import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;

import '../api/client.dart';
import '../api/models.dart';

class EmailStore extends ChangeNotifier {
  final _api = ApiClient.instance;

  bool connected = false;
  bool checked = false;
  String selfEmail = '';
  List<MailFolder> folders = [];
  MailFolder? folder;
  List<MessageSummary> messages = [];
  bool loading = false;
  bool loadingMore = false;
  bool exhausted = false;
  String? error;

  List<Provider> providers = [];
  String aiProvider = '';

  List<Suggestion> suggestions = [];
  Timer? _suggestionTimer;

  static const _folderOrder = [
    'Inbox', 'Drafts', 'Sent Items', 'Deleted Items', 'Junk Email',
  ];

  Future<void> init() async {
    if (checked) return;
    try {
      final cfg = await _api.getJson('/integrations/email/config');
      connected = cfg['connected'] == true;
      selfEmail = (cfg['connected_email'] as String?)?.toLowerCase() ?? '';
    } catch (_) {
      connected = false;
    }
    checked = true;
    notifyListeners();
    if (!connected) return;

    try {
      final pRes = await _api.getJson('/settings/providers');
      providers = (pRes['providers'] as List)
          .map((p) => Provider.fromJson(p as Map<String, dynamic>))
          .toList();
      if (providers.isNotEmpty) aiProvider = providers.first.name;
    } catch (_) {}
    await loadFolders();
    await loadSuggestions();
  }

  Future<void> loadFolders() async {
    try {
      final res = await _api.getJson('/email/folders');
      final all = (res['value'] as List)
          .map((f) => MailFolder.fromJson(f as Map<String, dynamic>))
          .toList();
      // Well-known folders first, in fixed order; the rest alphabetical.
      all.sort((a, b) {
        final ai = _folderOrder.indexOf(a.displayName);
        final bi = _folderOrder.indexOf(b.displayName);
        if (ai != -1 || bi != -1) {
          return (ai == -1 ? 99 : ai).compareTo(bi == -1 ? 99 : bi);
        }
        return a.displayName.compareTo(b.displayName);
      });
      folders = all;
      folder ??= folders.isNotEmpty ? folders.first : null;
      if (folder != null) await loadMessages();
    } catch (e) {
      error = e.toString();
    }
    notifyListeners();
  }

  Future<void> openFolder(MailFolder f) async {
    folder = f;
    messages = [];
    exhausted = false;
    notifyListeners();
    await loadMessages();
  }

  Future<void> loadMessages() async {
    final f = folder;
    if (f == null) return;
    loading = true;
    error = null;
    notifyListeners();
    try {
      final res = await _api
          .getJson('/email/folders/${f.id}/messages', {'skip': '0', 'top': '30'});
      messages = (res['value'] as List)
          .map((m) => MessageSummary.fromJson(m as Map<String, dynamic>))
          .toList();
      exhausted = messages.length < 30;
    } catch (e) {
      error = e.toString();
    } finally {
      loading = false;
      notifyListeners();
    }
  }

  Future<void> loadMore() async {
    final f = folder;
    if (f == null || loadingMore || exhausted) return;
    loadingMore = true;
    notifyListeners();
    try {
      final res = await _api.getJson('/email/folders/${f.id}/messages',
          {'skip': '${messages.length}', 'top': '30'});
      final more = (res['value'] as List)
          .map((m) => MessageSummary.fromJson(m as Map<String, dynamic>))
          .toList();
      exhausted = more.length < 30;
      messages.addAll(more);
    } catch (_) {
      // Scroll-triggered; silently retry on next reach.
    } finally {
      loadingMore = false;
      notifyListeners();
    }
  }

  Future<MessageDetail> getMessage(MessageSummary summary) async {
    final res = await _api.getJson('/email/messages/${summary.id}');
    if (!summary.isRead) {
      summary.isRead = true;
      if (folder != null && folder!.unread > 0) folder!.unread--;
      notifyListeners();
      // Fire and forget; list state is already updated optimistically.
      unawaited(_markRead(summary.id));
    }
    return MessageDetail.fromJson(res);
  }

  Future<void> _markRead(String id) async {
    try {
      await _api.patch('/email/messages/$id/read');
    } catch (_) {}
  }

  Future<List<Attachment>> listAttachments(String messageId) async {
    final res = await _api.getJson('/email/messages/$messageId/attachments');
    return (res['value'] as List)
        .map((a) => Attachment.fromJson(a as Map<String, dynamic>))
        .toList();
  }

  /// Fetch attachment bytes (cookie-authed) — used to inline cid: images.
  Future<(Uint8List, String)?> fetchAttachment(
      String messageId, Attachment att) async {
    try {
      final uri = Uri.parse(
          '${_api.baseUrl}/api/email/messages/${Uri.encodeComponent(messageId)}/attachments/${Uri.encodeComponent(att.id)}/raw');
      final res = await http.get(uri, headers: {
        if (_api.sessionCookie != null) 'Cookie': _api.sessionCookie!,
      });
      if (res.statusCode != 200) return null;
      return (res.bodyBytes, att.contentType);
    } catch (_) {
      return null;
    }
  }

  /// "No response needed" — complete the flag and file to Processed.
  Future<void> markDone(String messageId) async {
    await _api.postJson('/email/messages/$messageId/done', null);
    messages.removeWhere((m) => m.id == messageId);
    notifyListeners();
    // Refresh counts/folders in the background.
    Future<void>.delayed(const Duration(seconds: 1), loadFolders);
  }

  Future<void> send(Map<String, dynamic> payload) async {
    await _api.postJson('/email/send', payload);
    _pollSuggestions();
    // Replied mail gets filed server-side a moment later.
    Future<void>.delayed(const Duration(seconds: 3), loadFolders);
  }

  Stream<Map<String, dynamic>> streamAiDraft({
    required String from,
    required String subject,
    required String body,
  }) {
    return _api.streamSse('/email/ai-draft', {
      'provider': aiProvider,
      'from': from,
      'subject': subject,
      'body': body,
    });
  }

  Future<void> loadSuggestions() async {
    try {
      final res = await _api.getJson('/suggestions');
      suggestions = (res['suggestions'] as List)
          .map((s) => Suggestion.fromJson(s as Map<String, dynamic>))
          .toList();
      notifyListeners();
    } catch (_) {}
  }

  void _pollSuggestions() {
    _suggestionTimer?.cancel();
    var polls = 0;
    _suggestionTimer = Timer.periodic(const Duration(seconds: 3), (t) async {
      polls++;
      await loadSuggestions();
      if (polls >= 15) t.cancel();
    });
  }

  Future<void> acceptSuggestion(Suggestion s) async {
    suggestions.removeWhere((x) => x.id == s.id);
    notifyListeners();
    try {
      await _api.postJson('/suggestions/${s.id}/accept', null);
    } catch (e) {
      error = e.toString();
      notifyListeners();
    }
  }

  Future<void> dismissSuggestion(Suggestion s) async {
    suggestions.removeWhere((x) => x.id == s.id);
    notifyListeners();
    try {
      await _api.postJson('/suggestions/${s.id}/dismiss', null);
    } catch (_) {}
  }

  @override
  void dispose() {
    _suggestionTimer?.cancel();
    super.dispose();
  }
}
