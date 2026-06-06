import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../api/client.dart';
import '../api/models.dart';

class ChatStore extends ChangeNotifier {
  final _api = ApiClient.instance;

  List<Session> sessions = [];
  Session? active;
  List<ChatMessage> messages = [];
  List<PendingApproval> approvals = [];

  List<Provider> providers = [];
  String provider = '';

  bool sending = false;
  bool loading = false;
  String? error;

  StreamSubscription<Map<String, dynamic>>? _stream;
  int _localId = 0;

  /// First open: load providers + sessions, restore provider choice, and
  /// resume the most recent session (or start fresh).
  Future<void> init() async {
    if (sessions.isNotEmpty || loading) return;
    loading = true;
    notifyListeners();
    try {
      final prefs = await SharedPreferences.getInstance();
      final pRes = await _api.getJson('/settings/providers');
      providers = (pRes['providers'] as List)
          .map((p) => Provider.fromJson(p as Map<String, dynamic>))
          .toList();
      final saved = prefs.getString('chat_provider');
      provider = providers.any((p) => p.name == saved)
          ? saved!
          : (providers.isNotEmpty ? providers.first.name : '');

      final sRes = await _api.getJson('/sessions');
      sessions = (sRes['sessions'] as List)
          .map((s) => Session.fromJson(s as Map<String, dynamic>))
          .toList();
      if (sessions.isEmpty) {
        await newSession();
      } else {
        await openSession(sessions.first);
      }
    } catch (e) {
      error = e.toString();
    } finally {
      loading = false;
      notifyListeners();
    }
  }

  Future<void> setProvider(String name) async {
    provider = name;
    notifyListeners();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('chat_provider', name);
  }

  Future<void> newSession() async {
    final res = await _api.postJson('/sessions', {'title': 'New conversation'});
    final session = Session.fromJson(res['session'] as Map<String, dynamic>);
    sessions.insert(0, session);
    active = session;
    messages = [];
    approvals = [];
    notifyListeners();
  }

  Future<void> openSession(Session session) async {
    active = session;
    messages = [];
    approvals = [];
    notifyListeners();
    try {
      final res = await _api.getJson('/sessions/${session.id}/messages');
      messages = (res['messages'] as List)
          .map((m) => ChatMessage.fromJson(m as Map<String, dynamic>))
          .toList();
      final aRes = await _api.getJson('/sessions/${session.id}/approvals');
      approvals = (aRes['pending_actions'] as List)
          .map((a) => PendingApproval(
                id: a['id'] as String,
                toolName: a['tool_name'] as String,
                toolArgs: a['tool_args'] as String? ?? '{}',
              ))
          .toList();
    } catch (e) {
      error = e.toString();
    }
    notifyListeners();
  }

  String _nextId() => 'local-${_localId++}';

  /// `images`: `{mime, b64}` maps, matching the backend's multimodal shape.
  Future<void> send(String text, {List<Map<String, String>> images = const []}) async {
    final session = active;
    if (session == null || sending || (text.trim().isEmpty && images.isEmpty)) {
      return;
    }
    sending = true;
    error = null;
    // Local echo mirrors the DB shape so displayText/displayImages work on it.
    messages.add(ChatMessage(
      id: _nextId(),
      role: 'user',
      content: images.isEmpty
          ? text
          : jsonEncode({'type': 'multimodal', 'text': text, 'images': images}),
    ));
    notifyListeners();

    // Current assistant bubble; reset after each tool chip so text emitted
    // post-tool appears below the chip, matching the actual order of events.
    ChatMessage? current;
    try {
      final stream = _api.streamSse(
        '/sessions/${session.id}/chat',
        {'message': text, 'provider': provider, 'images': images},
      );
      _stream = stream.listen(
        (event) {
          switch (event['type']) {
            case 'token':
              if (current == null) {
                current =
                    ChatMessage(id: _nextId(), role: 'assistant', content: '');
                messages.add(current!);
              }
              current!.content += event['text'] as String? ?? '';
            case 'tool':
              messages.add(ChatMessage(
                id: _nextId(),
                role: 'tool_call',
                content: event['name'] as String? ?? '',
              ));
              current = null;
            case 'awaiting_approval':
              approvals.add(PendingApproval(
                id: event['action_id'] as String,
                toolName: event['tool_name'] as String? ?? '',
                toolArgs: jsonEncode(event['tool_args'] ?? {}),
              ));
            case 'done':
              sending = false;
          }
          notifyListeners();
        },
        onDone: () {
          sending = false;
          notifyListeners();
        },
        onError: (Object e) {
          error = e.toString();
          sending = false;
          notifyListeners();
        },
      );
      await _stream!.asFuture<void>();
    } catch (e) {
      error = e.toString();
      sending = false;
      notifyListeners();
    }
  }

  void cancel() {
    _stream?.cancel();
    _stream = null;
    sending = false;
    notifyListeners();
  }

  Future<void> approve(PendingApproval a) => _decide(a, true);
  Future<void> reject(PendingApproval a) => _decide(a, false);

  Future<void> _decide(PendingApproval a, bool approved) async {
    approvals.removeWhere((x) => x.id == a.id);
    notifyListeners();
    try {
      await _api.postJson(
          '/approvals/${a.id}/${approved ? 'approve' : 'reject'}', null);
    } catch (e) {
      error = e.toString();
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _stream?.cancel();
    super.dispose();
  }
}
