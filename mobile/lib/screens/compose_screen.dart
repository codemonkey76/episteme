import 'dart:async';

import 'package:flutter/material.dart';
import 'package:provider/provider.dart' hide Provider;

import '../api/models.dart';
import '../main.dart';
import '../state/email.dart';

/// Full-screen composer. `mode`: 'reply' | 'replyAll' | 'forward' | 'ai'
/// (= reply with an AI-streamed draft pre-filled).
Future<void> showComposeScreen(
  BuildContext context, {
  required MessageDetail detail,
  required String mode,
  required String latestText,
}) {
  return Navigator.of(context).push(MaterialPageRoute(
    fullscreenDialog: true,
    builder: (_) => ComposeScreen(
      detail: detail,
      mode: mode,
      latestText: latestText,
    ),
  ));
}

class ComposeScreen extends StatefulWidget {
  const ComposeScreen({
    super.key,
    required this.detail,
    required this.mode,
    required this.latestText,
  });

  final MessageDetail detail;
  final String mode;
  final String latestText;

  @override
  State<ComposeScreen> createState() => _ComposeScreenState();
}

class _ComposeScreenState extends State<ComposeScreen> {
  final _to = TextEditingController();
  final _body = TextEditingController();
  String _aiDraftOriginal = '';
  bool _drafting = false;
  bool _sending = false;
  String? _status;
  StreamSubscription<Map<String, dynamic>>? _draftStream;

  bool get _isReplyAll => widget.mode == 'replyAll';
  bool get _isForward => widget.mode == 'forward';

  String get _subject {
    final s = widget.detail.summary.subject;
    if (_isForward) return s.startsWith('Fwd:') ? s : 'Fwd: $s';
    return s.startsWith('Re:') ? s : 'Re: $s';
  }

  List<String> get _toList => _to.text
      .split(',')
      .map((a) => a.trim())
      .where((a) => a.isNotEmpty)
      .toList();

  /// Reply-all: everyone on the original (To + Cc) except the sender and us.
  List<String> _ccFor(String selfEmail) {
    if (!_isReplyAll) return const [];
    final sender = widget.detail.summary.from.address.toLowerCase();
    return [...widget.detail.to, ...widget.detail.cc]
        .map((a) => a.address)
        .where((a) =>
            a.isNotEmpty &&
            a.toLowerCase() != sender &&
            a.toLowerCase() != selfEmail)
        .toSet()
        .toList();
  }

  @override
  void initState() {
    super.initState();
    if (!_isForward) _to.text = widget.detail.summary.from.address;
    if (widget.mode == 'ai') _draft();
  }

  Future<void> _draft() async {
    final store = context.read<EmailStore>();
    if (store.aiProvider.isEmpty) {
      setState(() => _status = 'No AI provider configured.');
      return;
    }
    setState(() {
      _drafting = true;
      _status = 'Drafting…';
      _body.text = '';
    });
    try {
      final s = widget.detail.summary;
      _draftStream = store
          .streamAiDraft(
            from: '${s.from.name} <${s.from.address}>',
            subject: s.subject,
            body: widget.latestText,
          )
          .listen((event) {
        if (event['type'] == 'token') {
          _body.text += event['text'] as String? ?? '';
        } else if (event['type'] == 'error') {
          setState(() => _status = 'Draft failed: ${event['message']}');
        }
      });
      await _draftStream!.asFuture<void>();
      _aiDraftOriginal = _body.text;
      if (mounted) setState(() => _status = null);
    } catch (e) {
      if (mounted) setState(() => _status = 'Draft failed: $e');
    } finally {
      if (mounted) setState(() => _drafting = false);
    }
  }

  Future<void> _send() async {
    final store = context.read<EmailStore>();
    final body = _body.text.trim();
    if (body.isEmpty || _sending) return;
    setState(() {
      _sending = true;
      _status = 'Sending…';
    });
    try {
      final to = _toList;
      if (to.isEmpty) {
        setState(() {
          _sending = false;
          _status = 'Add a recipient.';
        });
        return;
      }
      await store.send({
        'to': to,
        'cc': _ccFor(store.selfEmail),
        'body': body,
        'reply_to_message_id': widget.detail.summary.id,
        'action': _isForward
            ? 'forward'
            : (_isReplyAll ? 'replyAll' : 'reply'),
        'subject': _subject,
        'reply_context': widget.latestText.length > 2000
            ? widget.latestText.substring(0, 2000)
            : widget.latestText,
        if (store.aiProvider.isNotEmpty) 'ai_provider': store.aiProvider,
        if (_aiDraftOriginal.isNotEmpty) 'ai_draft': _aiDraftOriginal,
      });
      if (mounted) Navigator.pop(context);
    } catch (e) {
      if (mounted) {
        setState(() {
          _sending = false;
          _status = 'Send failed: $e';
        });
      }
    }
  }

  @override
  void dispose() {
    _draftStream?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final cc = _ccFor(context.read<EmailStore>().selfEmail);
    final title = _isForward
        ? 'Forward'
        : _isReplyAll
            ? 'Reply all'
            : 'Reply';
    return Scaffold(
      backgroundColor: Palette.bg,
      appBar: AppBar(
        title: Text(title, style: const TextStyle(fontSize: 16)),
        actions: [
          if (widget.mode != 'ai' && !_isForward)
            TextButton.icon(
              onPressed: _drafting ? null : _draft,
              icon: const Icon(Icons.auto_awesome,
                  size: 15, color: Color(0xFFA0C8FF)),
              label: const Text('AI draft',
                  style: TextStyle(color: Color(0xFFA0C8FF), fontSize: 13)),
            ),
          Padding(
            padding: const EdgeInsets.only(right: 8),
            child: FilledButton.icon(
              style: FilledButton.styleFrom(
                backgroundColor: Palette.accentBg,
                foregroundColor: Palette.accent,
                visualDensity: VisualDensity.compact,
              ),
              onPressed: _sending || _drafting ? null : _send,
              icon: _sending
                  ? const SizedBox(
                      width: 13,
                      height: 13,
                      child: CircularProgressIndicator(strokeWidth: 2))
                  : const Icon(Icons.send, size: 15),
              label: const Text('Send'),
            ),
          ),
        ],
      ),
      body: ListView(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 24),
        children: [
          Text(_subject,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(
                  color: Palette.fg,
                  fontSize: 14.5,
                  fontWeight: FontWeight.w600)),
          const SizedBox(height: 10),
          TextField(
            controller: _to,
            enabled: _isForward,
            keyboardType: TextInputType.emailAddress,
            autocorrect: false,
            decoration: InputDecoration(
              labelText: 'To',
              hintText: _isForward ? 'name@example.com, …' : null,
              isDense: true,
            ),
          ),
          if (cc.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(top: 6),
              child: Text('cc ${cc.join(', ')}',
                  style:
                      const TextStyle(color: Palette.faint, fontSize: 12)),
            ),
          const SizedBox(height: 12),
          TextField(
            controller: _body,
            minLines: 10,
            maxLines: null,
            autofocus: widget.mode != 'ai',
            decoration: InputDecoration(
              hintText: _drafting
                  ? 'AI is drafting…'
                  : _isForward
                      ? 'Add a comment… (the original message is included automatically)'
                      : 'Write your reply…',
            ),
          ),
          if (_status != null) ...[
            const SizedBox(height: 8),
            Text(_status!,
                style: TextStyle(
                    fontSize: 12.5,
                    color: _status!.contains('failed') ||
                            _status!.contains('recipient')
                        ? Palette.danger
                        : Palette.faint)),
          ],
        ],
      ),
    );
  }
}
