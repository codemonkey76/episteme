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

  /// Per-mailbox HTML signatures ('' = own mailbox), appended to outgoing
  /// bodies — the plain-text composer can't show them inline like the web.
  Map<String, String> signatures = {};
  String get signature => signatures[currentMailbox] ?? '';

  /// Shared mailboxes the user added (web Settings → Integrations).
  /// '' = the user's own mailbox; otherwise the shared address. Threaded as
  /// ?mailbox= / payload field into every Graph-backed call.
  List<SharedMailbox> sharedMailboxes = [];
  String currentMailbox = '';

  /// Connected Microsoft 365 instances the user can read from. Each call is
  /// scoped by `account=<id>` (which connection) and, within it, `mailbox=`
  /// (a shared mailbox; '' = that account's own mailbox).
  List<EmailAccount> accounts = [];
  String accountId = '';

  /// Display label for the active account (its name, or email as a fallback).
  String get currentAccountLabel => accounts
      .firstWhere((a) => a.id == accountId,
          orElse: () => EmailAccount(id: '', name: '', email: selfEmail))
      .label;

  /// account + mailbox selectors, shared by every Graph-backed call. GET
  /// requests pass them as query params; POST/PATCH bodies as fields; the
  /// path-built helpers append them as a query string.
  Map<String, String> get _sel => {
        if (accountId.isNotEmpty) 'account': accountId,
        if (currentMailbox.isNotEmpty) 'mailbox': currentMailbox,
      };

  /// Query-map entry for GET requests.
  Map<String, String> get _mbParams => _sel;

  /// `?…` suffix for path-built requests (POST/PATCH/raw helpers have no
  /// query-parameter argument).
  String get _mbq {
    final p = _sel;
    if (p.isEmpty) return '';
    return '?${p.entries.map((e) => '${e.key}=${Uri.encodeQueryComponent(e.value)}').join('&')}';
  }

  Future<void> switchMailbox(String address) async {
    if (address == currentMailbox) return;
    currentMailbox = address;
    folders = [];
    folder = null;
    messages = [];
    exhausted = false;
    error = null;
    notifyListeners();
    await loadFolders();
  }

  /// Switch to another connected Microsoft 365 account. Resets to its own
  /// mailbox and reloads its shared mailboxes, signatures, and folders.
  Future<void> switchAccount(String id) async {
    if (id == accountId || id.isEmpty) return;
    accountId = id;
    selfEmail = accounts
            .firstWhere((a) => a.id == id,
                orElse: () => EmailAccount(id: id, name: '', email: ''))
            .email
            .toLowerCase();
    currentMailbox = '';
    folders = [];
    folder = null;
    messages = [];
    sharedMailboxes = [];
    exhausted = false;
    error = null;
    notifyListeners();
    await _loadSignatures();
    await _loadShared();
    await loadFolders();
  }

  Future<void> _loadSignatures() async {
    try {
      final sigs = await _api.getJson('/email/signatures');
      signatures = sigs.map((k, v) => MapEntry(k, (v as String?) ?? ''));
    } catch (_) {}
  }

  Future<void> _loadShared() async {
    try {
      final res = await _api.getJson('/integrations/$accountId/shared');
      sharedMailboxes = (res['mailboxes'] as List)
          .map((m) => SharedMailbox.fromJson(m as Map<String, dynamic>))
          .toList();
    } catch (_) {
      sharedMailboxes = [];
    }
  }

  List<Suggestion> suggestions = [];
  Timer? _suggestionTimer;

  static const _folderOrder = [
    'Inbox', 'Drafts', 'Sent Items', 'Deleted Items', 'Junk Email',
  ];

  Future<void> init() async {
    if (checked) return;
    try {
      final res = await _api.getJson('/integrations');
      final msft = (res['integrations'] as List? ?? const [])
          .cast<Map<String, dynamic>>()
          .where((i) => i['kind'] == 'microsoft')
          .toList();
      // Only connected instances are usable; let the user switch between them.
      accounts = msft
          .where((i) => i['connected'] == true)
          .map(EmailAccount.fromJson)
          .toList();
      connected = accounts.isNotEmpty;
      if (connected) {
        // Read from the default instance when it's connected, else the first.
        final def = msft.firstWhere(
          (i) => i['is_default'] == true && i['connected'] == true,
          orElse: () => <String, dynamic>{},
        );
        final chosen = def.isNotEmpty ? EmailAccount.fromJson(def) : accounts.first;
        accountId = chosen.id;
        selfEmail = chosen.email.toLowerCase();
      }
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
    await _loadSignatures();
    await _loadShared();
    await loadFolders();
    await loadSuggestions();
  }

  Future<void> loadFolders() async {
    try {
      final res = await _api.getJson('/email/folders', _mbParams);
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
      final res = await _api.getJson('/email/folders/${f.id}/messages',
          {'skip': '0', 'top': '30', ..._mbParams});
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
          {'skip': '${messages.length}', 'top': '30', ..._mbParams});
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
    final res = await _api.getJson('/email/messages/${summary.id}', _mbParams);
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
      await _api.patch('/email/messages/$id/read$_mbq');
    } catch (_) {}
  }

  Future<List<Attachment>> listAttachments(String messageId) async {
    final res = await _api.getJson(
        '/email/messages/$messageId/attachments', _mbParams);
    return (res['value'] as List)
        .map((a) => Attachment.fromJson(a as Map<String, dynamic>))
        .toList();
  }

  /// Fetch attachment bytes (cookie-authed) — used to inline cid: images.
  Future<(Uint8List, String)?> fetchAttachment(
      String messageId, Attachment att) async {
    try {
      final uri = Uri.parse(
          '${_api.baseUrl}/api/email/messages/${Uri.encodeComponent(messageId)}/attachments/${Uri.encodeComponent(att.id)}/raw$_mbq');
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
    await _api.postJson('/email/messages/$messageId/done$_mbq', null);
    messages.removeWhere((m) => m.id == messageId);
    notifyListeners();
    // Refresh counts/folders in the background.
    Future<void>.delayed(const Duration(seconds: 1), loadFolders);
  }

  /// Soft delete — move the messages to Deleted Items.
  Future<int> deleteMessages(List<String> ids) async {
    final res = await _api.postJson('/email/messages/delete', {
      'ids': ids,
      ..._sel,
    });
    messages.removeWhere((m) => ids.contains(m.id));
    notifyListeners();
    Future<void>.delayed(const Duration(seconds: 1), loadFolders);
    return (res['deleted'] as num?)?.toInt() ?? ids.length;
  }

  /// Create a helpdesk ticket from the email (sender matched server-side).
  Future<Map<String, dynamic>> createTicket(String messageId) =>
      _api.postJson('/email/messages/$messageId/ticket', {
        'provider': aiProvider,
        ..._sel,
      });

  /// Stream a short AI summary of the email (token/error/done events).
  Stream<Map<String, dynamic>> streamSummary(String messageId) =>
      _api.streamSse('/email/messages/$messageId/summarize', {
        'provider': aiProvider,
        ..._sel,
      });

  Future<void> send(Map<String, dynamic> payload) async {
    payload.addAll(_sel);
    await _api.postJson('/email/send', payload);
    _pollSuggestions();
    // Replied mail gets filed server-side a moment later.
    Future<void>.delayed(const Duration(seconds: 3), loadFolders);
  }

  Stream<Map<String, dynamic>> streamAiDraft({
    required String from,
    required String subject,
    required String body,
    String? instruction,
  }) {
    return _api.streamSse('/email/ai-draft', {
      'provider': aiProvider,
      'from': from,
      'subject': subject,
      'body': body,
      if (instruction != null && instruction.trim().isNotEmpty)
        'instruction': instruction.trim(),
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
