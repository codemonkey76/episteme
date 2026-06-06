import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:mime/mime.dart';
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

class _PendingAttachment {
  _PendingAttachment(this.name, this.mime, this.bytes);
  final String name;
  final String mime;
  final Uint8List bytes;
}

class _ComposeScreenState extends State<ComposeScreen> {
  final _to = TextEditingController();
  final _body = TextEditingController();
  String _aiDraftOriginal = '';
  bool _drafting = false;
  bool _sending = false;
  String? _status;
  StreamSubscription<Map<String, dynamic>>? _draftStream;

  /// Outlook's message cap; the backend chunks big files via upload sessions.
  static const _maxAttachTotal = 35 * 1024 * 1024;
  final List<_PendingAttachment> _attachments = [];

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

  Future<void> _pickFiles({bool images = false}) async {
    final result = await FilePicker.platform.pickFiles(
      allowMultiple: true,
      withData: true,
      type: images ? FileType.image : FileType.any,
    );
    if (result == null || !mounted) return;
    for (final f in result.files) {
      final bytes = f.bytes;
      if (bytes == null) continue;
      final total = _attachments.fold<int>(0, (s, a) => s + a.bytes.length);
      if (total + bytes.length > _maxAttachTotal) {
        _status = 'Skipped ${f.name} — attachments are capped at 35 MB total.';
        continue;
      }
      _attachments.add(_PendingAttachment(
        f.name,
        lookupMimeType(f.name) ?? 'application/octet-stream',
        bytes,
      ));
    }
    setState(() {});
  }

  /// The backend now treats `body` as HTML (the web composer is rich text) —
  /// escape and convert newlines so plain text survives the trip.
  static String _textToHtml(String text) {
    final esc = text
        .replaceAll('&', '&amp;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;');
    return esc
        .split('\n')
        .map((l) => l.trim().isEmpty ? '<div><br></div>' : '<div>$l</div>')
        .join();
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
      // Signature rides along as HTML below the message (the plain-text
      // composer can't render it inline like the web's rich editor).
      final sig = store.signature.trim();
      await store.send({
        'to': to,
        'cc': _ccFor(store.selfEmail),
        'body': _textToHtml(body) +
            (sig.isEmpty ? '' : '<div><br></div><div><br></div>$sig'),
        if (_attachments.isNotEmpty)
          'attachments': _attachments
              .map((a) => {
                    'name': a.name,
                    'content_type': a.mime,
                    'content_bytes': base64Encode(a.bytes),
                    'is_inline': false,
                  })
              .toList(),
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
          if (_attachments.isNotEmpty) ...[
            const SizedBox(height: 10),
            Wrap(
              spacing: 6,
              runSpacing: 6,
              children: [
                for (var i = 0; i < _attachments.length; i++)
                  Chip(
                    backgroundColor: Palette.surface,
                    side: const BorderSide(color: Palette.raised),
                    label: Text(
                      '${_attachments[i].name} · ${_fmtSize(_attachments[i].bytes.length)}',
                      style:
                          const TextStyle(color: Palette.fg, fontSize: 12),
                    ),
                    deleteIcon: const Icon(Icons.close,
                        size: 15, color: Palette.muted),
                    onDeleted: () =>
                        setState(() => _attachments.removeAt(i)),
                  ),
              ],
            ),
          ],
          const SizedBox(height: 8),
          Row(
            children: [
              TextButton.icon(
                onPressed: _sending ? null : () => _pickFiles(),
                icon: const Icon(Icons.attach_file,
                    size: 15, color: Palette.muted),
                label: const Text('Attach file',
                    style: TextStyle(color: Palette.muted, fontSize: 12.5)),
              ),
              TextButton.icon(
                onPressed: _sending ? null : () => _pickFiles(images: true),
                icon: const Icon(Icons.image_outlined,
                    size: 15, color: Palette.muted),
                label: const Text('Photo',
                    style: TextStyle(color: Palette.muted, fontSize: 12.5)),
              ),
            ],
          ),
          if (context.read<EmailStore>().signature.trim().isNotEmpty)
            const Padding(
              padding: EdgeInsets.only(top: 4),
              child: Text('Your signature is added automatically.',
                  style: TextStyle(color: Palette.faint, fontSize: 11.5)),
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

  static String _fmtSize(int bytes) {
    if (bytes < 1024) return '$bytes B';
    if (bytes < 1024 * 1024) return '${(bytes / 1024).round()} KB';
    return '${(bytes / 1024 / 1024).toStringAsFixed(1)} MB';
  }
}
