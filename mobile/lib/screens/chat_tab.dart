import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:path_provider/path_provider.dart';
import 'package:provider/provider.dart' hide Provider;
import 'package:record/record.dart';

import '../api/client.dart';
import '../api/models.dart';
import '../main.dart';
import '../state/chat.dart';

const _toolLabels = {
  'create_calendar_event': 'Creating calendar event',
  'list_calendar_events': 'Checking the calendar',
  'delete_calendar_event': 'Deleting calendar event',
  'list_tasks': 'Checking the to-do list',
  'create_task': 'Adding task',
  'update_task': 'Updating task',
  'complete_task': 'Completing task',
  'delete_task': 'Deleting task',
  'list_notes': 'Searching notes',
  'get_note': 'Reading note',
  'create_note': 'Saving note',
  'update_note': 'Updating note',
  'delete_note': 'Deleting note',
};

String toolLabel(String name) => _toolLabels[name] ?? 'Using $name';

/// Renders tool-call chips as "Label: detail, Label…", with the detail dimmed.
List<TextSpan> toolChipSpans(List<ToolChip> chips) {
  final spans = <TextSpan>[];
  for (var i = 0; i < chips.length; i++) {
    if (i > 0) spans.add(const TextSpan(text: ', '));
    spans.add(TextSpan(text: toolLabel(chips[i].name)));
    if (chips[i].detail.isNotEmpty) {
      spans.add(TextSpan(
        text: ': ${chips[i].detail}',
        style: const TextStyle(color: Color(0xFF5E7186)),
      ));
    }
  }
  spans.add(const TextSpan(text: '…'));
  return spans;
}

class ChatTab extends StatefulWidget {
  const ChatTab({super.key});

  @override
  State<ChatTab> createState() => _ChatTabState();
}

class _ChatTabState extends State<ChatTab> {
  final _input = TextEditingController();
  final _scroll = ScrollController();

  static const _maxImages = 4;
  /// Pending attachments: `{mime, b64}` maps the backend accepts directly.
  final List<Map<String, String>> _pendingImages = [];

  Future<void> _pickImages() async {
    final result = await FilePicker.platform.pickFiles(
      type: FileType.image,
      allowMultiple: true,
      withData: true,
    );
    if (result == null) return;
    setState(() {
      for (final f in result.files) {
        if (_pendingImages.length >= _maxImages) break;
        final bytes = f.bytes;
        if (bytes == null) continue;
        final ext = (f.extension ?? 'png').toLowerCase();
        final mime = ext == 'jpg' ? 'image/jpeg' : 'image/$ext';
        _pendingImages.add({'mime': mime, 'b64': base64Encode(bytes)});
      }
    });
  }

  // ── Voice input ─────────────────────────────────────────────────────────
  final _recorder = AudioRecorder();
  bool _recording = false;
  bool _transcribing = false;

  Future<void> _toggleRecording() async {
    if (_recording) {
      final path = await _recorder.stop();
      setState(() => _recording = false);
      if (path == null) return;
      setState(() => _transcribing = true);
      try {
        final bytes = await File(path).readAsBytes();
        final res = await ApiClient.instance.postJson('/transcribe', {
          'audio_b64': base64Encode(bytes),
          'mime': 'audio/m4a',
        });
        final text = (res['text'] as String? ?? '').trim();
        if (text.isNotEmpty) {
          _input.text = _input.text.isEmpty ? text : '${_input.text} $text';
        }
      } catch (e) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text('Transcription failed: $e')),
          );
        }
      } finally {
        try {
          await File(path).delete();
        } catch (_) {}
        if (mounted) setState(() => _transcribing = false);
      }
      return;
    }
    if (!await _recorder.hasPermission()) return;
    final dir = await getTemporaryDirectory();
    await _recorder.start(
      const RecordConfig(encoder: AudioEncoder.aacLc),
      path: '${dir.path}/voice-${DateTime.now().millisecondsSinceEpoch}.m4a',
    );
    setState(() => _recording = true);
  }

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<ChatStore>().init();
    });
  }

  @override
  void dispose() {
    _recorder.dispose();
    super.dispose();
  }

  void _autoscroll() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_scroll.hasClients) {
        _scroll.jumpTo(_scroll.position.maxScrollExtent);
      }
    });
  }

  Future<void> _send() async {
    final store = context.read<ChatStore>();
    final text = _input.text.trim();
    if ((text.isEmpty && _pendingImages.isEmpty) || store.sending) return;
    final images = List<Map<String, String>>.from(_pendingImages);
    _input.clear();
    setState(_pendingImages.clear);
    await store.send(text, images: images);
  }

  void _showSessions() {
    final store = context.read<ChatStore>();
    showModalBottomSheet(
      context: context,
      backgroundColor: Palette.surface,
      shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(16))),
      builder: (sheetCtx) => SafeArea(
        child: ListView(
          shrinkWrap: true,
          children: [
            ListTile(
              leading: const Icon(Icons.add, color: Palette.accent),
              title: const Text('New conversation',
                  style: TextStyle(color: Palette.accent)),
              onTap: () async {
                Navigator.pop(sheetCtx);
                await store.newSession();
              },
            ),
            ...store.sessions.map((s) => ListTile(
                  leading: Icon(
                    Icons.chat_bubble_outline,
                    size: 18,
                    color: s.id == store.active?.id
                        ? Palette.accent
                        : Palette.faint,
                  ),
                  title: Text(s.title,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                          fontSize: 14,
                          color: s.id == store.active?.id
                              ? Palette.fg
                              : Palette.muted)),
                  onTap: () {
                    Navigator.pop(sheetCtx);
                    store.openSession(s);
                  },
                )),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<ChatStore>();
    _autoscroll();

    final visible =
        store.messages.where((m) => m.role != 'tool').toList(growable: false);

    return Scaffold(
      backgroundColor: Palette.bg,
      body: Column(
        children: [
          // Session bar
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 4, 8, 0),
            child: Row(
              children: [
                Expanded(
                  child: Text(store.active?.title ?? '',
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style:
                          const TextStyle(color: Palette.faint, fontSize: 12.5)),
                ),
                IconButton(
                  tooltip: 'Conversations',
                  icon: const Icon(Icons.history, size: 19, color: Palette.muted),
                  onPressed: _showSessions,
                ),
                IconButton(
                  tooltip: 'New conversation',
                  icon: const Icon(Icons.add_comment_outlined,
                      size: 19, color: Palette.muted),
                  onPressed: store.sending ? null : store.newSession,
                ),
              ],
            ),
          ),
          Expanded(
            child: store.loading
                ? const Center(child: CircularProgressIndicator())
                : ListView.builder(
                    controller: _scroll,
                    padding: const EdgeInsets.fromLTRB(12, 4, 12, 12),
                    itemCount: visible.length + store.approvals.length,
                    itemBuilder: (_, i) {
                      if (i < visible.length) {
                        return _MessageRow(message: visible[i]);
                      }
                      return _ApprovalCard(
                          approval: store.approvals[i - visible.length]);
                    },
                  ),
          ),
          if (store.error != null)
            Container(
              width: double.infinity,
              color: const Color(0xFF5A2A2A),
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 6),
              child: Text(store.error!,
                  style: const TextStyle(color: Color(0xFFE0C0C0), fontSize: 12)),
            ),
          if (_pendingImages.isNotEmpty)
            Container(
              height: 64,
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
              alignment: Alignment.centerLeft,
              child: ListView.separated(
                scrollDirection: Axis.horizontal,
                itemCount: _pendingImages.length,
                separatorBuilder: (_, _) => const SizedBox(width: 6),
                itemBuilder: (_, i) => Stack(
                  children: [
                    ClipRRect(
                      borderRadius: BorderRadius.circular(6),
                      child: Image.memory(
                        base64Decode(_pendingImages[i]['b64']!),
                        width: 56,
                        height: 56,
                        fit: BoxFit.cover,
                      ),
                    ),
                    Positioned(
                      top: 0,
                      right: 0,
                      child: GestureDetector(
                        onTap: () => setState(() => _pendingImages.removeAt(i)),
                        child: Container(
                          padding: const EdgeInsets.all(2),
                          decoration: const BoxDecoration(
                            color: Color(0xCC000000),
                            shape: BoxShape.circle,
                          ),
                          child: const Icon(Icons.close, size: 12, color: Colors.white),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          _InputBar(
            input: _input,
            onSend: _send,
            onAttach: _pickImages,
            onMic: _toggleRecording,
            recording: _recording,
            transcribing: _transcribing,
          ),
        ],
      ),
    );
  }
}

class _MessageRow extends StatelessWidget {
  const _MessageRow({required this.message});
  final ChatMessage message;

  @override
  Widget build(BuildContext context) {
    switch (message.role) {
      case 'tool_call':
        return Padding(
          padding: const EdgeInsets.symmetric(vertical: 3),
          child: Row(
            children: [
              const Icon(Icons.build_outlined, size: 13, color: Color(0xFF7A9EC0)),
              const SizedBox(width: 6),
              Flexible(
                child: Text.rich(
                  TextSpan(children: toolChipSpans(message.toolChips)),
                  style: const TextStyle(color: Color(0xFF7A9EC0), fontSize: 12.5),
                ),
              ),
            ],
          ),
        );
      case 'user':
        final images = message.displayImages;
        return Align(
          alignment: Alignment.centerRight,
          child: Container(
            margin: const EdgeInsets.symmetric(vertical: 4),
            padding: const EdgeInsets.symmetric(horizontal: 13, vertical: 9),
            constraints: const BoxConstraints(maxWidth: 320),
            decoration: BoxDecoration(
              color: const Color(0xFF1E2A3A),
              borderRadius: BorderRadius.circular(12),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              mainAxisSize: MainAxisSize.min,
              children: [
                if (images.isNotEmpty)
                  Wrap(
                    spacing: 6,
                    runSpacing: 6,
                    children: images
                        .map<Widget>((Uint8List bytes) => ClipRRect(
                              borderRadius: BorderRadius.circular(8),
                              child: Image.memory(bytes,
                                  width: 180, fit: BoxFit.contain),
                            ))
                        .toList(),
                  ),
                if (images.isNotEmpty && message.displayText.isNotEmpty)
                  const SizedBox(height: 6),
                if (message.displayText.isNotEmpty)
                  Text(message.displayText,
                      style: const TextStyle(
                          color: Palette.fg, fontSize: 14.5, height: 1.4)),
              ],
            ),
          ),
        );
      default: // assistant
        if (message.displayText.trim().isEmpty) return const SizedBox.shrink();
        return Align(
          alignment: Alignment.centerLeft,
          child: Container(
            margin: const EdgeInsets.symmetric(vertical: 4),
            padding: const EdgeInsets.symmetric(horizontal: 13, vertical: 9),
            decoration: BoxDecoration(
              color: Palette.surface,
              borderRadius: BorderRadius.circular(12),
            ),
            child: MarkdownBody(
              data: message.displayText,
              styleSheet: MarkdownStyleSheet(
                p: const TextStyle(color: Palette.fg, fontSize: 14.5, height: 1.45),
                code: const TextStyle(
                    backgroundColor: Color(0xFF181818),
                    color: Palette.fg,
                    fontSize: 13),
                codeblockDecoration: BoxDecoration(
                  color: const Color(0xFF0D0D0D),
                  borderRadius: BorderRadius.circular(6),
                ),
                listBullet: const TextStyle(color: Palette.fg, fontSize: 14.5),
                h1: const TextStyle(color: Palette.fg, fontSize: 18, fontWeight: FontWeight.w600),
                h2: const TextStyle(color: Palette.fg, fontSize: 16.5, fontWeight: FontWeight.w600),
                h3: const TextStyle(color: Palette.fg, fontSize: 15.5, fontWeight: FontWeight.w600),
                blockquote: const TextStyle(color: Palette.muted),
                a: const TextStyle(color: Palette.accent),
              ),
            ),
          ),
        );
    }
  }
}

class _ApprovalCard extends StatelessWidget {
  const _ApprovalCard({required this.approval});
  final PendingApproval approval;

  @override
  Widget build(BuildContext context) {
    final store = context.read<ChatStore>();
    return Container(
      margin: const EdgeInsets.symmetric(vertical: 6),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: const Color(0xFF1A1610),
        border: Border.all(color: const Color(0xFF4A3A1A)),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              const Icon(Icons.warning_amber_outlined,
                  size: 15, color: Palette.warn),
              const SizedBox(width: 6),
              Expanded(
                child: Text('${toolLabel(approval.toolName)} — approval required',
                    style: const TextStyle(
                        color: Palette.warn,
                        fontSize: 13,
                        fontWeight: FontWeight.w500)),
              ),
            ],
          ),
          const SizedBox(height: 8),
          Container(
            width: double.infinity,
            padding: const EdgeInsets.all(8),
            constraints: const BoxConstraints(maxHeight: 140),
            decoration: BoxDecoration(
              color: const Color(0xFF12100A),
              borderRadius: BorderRadius.circular(6),
            ),
            child: SingleChildScrollView(
              child: Text(approval.prettyArgs,
                  style: const TextStyle(
                      color: Color(0xFFA09070),
                      fontSize: 11.5,
                      fontFamily: 'monospace')),
            ),
          ),
          const SizedBox(height: 10),
          Row(
            children: [
              FilledButton(
                style: FilledButton.styleFrom(
                  backgroundColor: const Color(0xFF1E3A2A),
                  foregroundColor: Palette.ok,
                  visualDensity: VisualDensity.compact,
                ),
                onPressed: () => store.approve(approval),
                child: const Text('Approve'),
              ),
              const SizedBox(width: 8),
              FilledButton(
                style: FilledButton.styleFrom(
                  backgroundColor: const Color(0xFF3A1E1E),
                  foregroundColor: Palette.danger,
                  visualDensity: VisualDensity.compact,
                ),
                onPressed: () => store.reject(approval),
                child: const Text('Deny'),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

class _InputBar extends StatelessWidget {
  const _InputBar({
    required this.input,
    required this.onSend,
    required this.onAttach,
    required this.onMic,
    required this.recording,
    required this.transcribing,
  });
  final TextEditingController input;
  final Future<void> Function() onSend;
  final Future<void> Function() onAttach;
  final Future<void> Function() onMic;
  final bool recording;
  final bool transcribing;

  @override
  Widget build(BuildContext context) {
    final store = context.watch<ChatStore>();
    return SafeArea(
      top: false,
      child: Container(
        padding: const EdgeInsets.fromLTRB(10, 8, 10, 10),
        decoration: const BoxDecoration(
          border: Border(top: BorderSide(color: Color(0xFF1E1E1E))),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Row(
              children: [
                IconButton(
                  tooltip: 'Attach image',
                  icon: const Icon(Icons.image_outlined, size: 20, color: Palette.muted),
                  onPressed: store.sending ? null : onAttach,
                ),
                IconButton(
                  tooltip: recording ? 'Stop recording' : 'Voice input',
                  icon: transcribing
                      ? const SizedBox(
                          width: 18,
                          height: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : Icon(
                          recording ? Icons.stop_circle_outlined : Icons.mic_none,
                          size: 20,
                          color: recording ? Palette.danger : Palette.muted,
                        ),
                  onPressed: (store.sending || transcribing) ? null : onMic,
                ),
                Expanded(
                  child: TextField(
                    controller: input,
                    minLines: 1,
                    maxLines: 5,
                    textInputAction: TextInputAction.newline,
                    decoration: const InputDecoration(
                      hintText: 'Message…',
                      isDense: true,
                      contentPadding:
                          EdgeInsets.symmetric(horizontal: 12, vertical: 10),
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                store.sending
                    ? IconButton.filled(
                        style: IconButton.styleFrom(
                            backgroundColor: const Color(0xFF5A2A2A)),
                        icon: const Icon(Icons.stop, size: 18),
                        onPressed: store.cancel,
                      )
                    : IconButton.filled(
                        style: IconButton.styleFrom(
                            backgroundColor: Palette.accentBg,
                            foregroundColor: Palette.accent),
                        icon: const Icon(Icons.send, size: 18),
                        onPressed: onSend,
                      ),
              ],
            ),
            if (store.providers.length > 1)
              Align(
                alignment: Alignment.centerLeft,
                child: DropdownButton<String>(
                  value: store.provider.isEmpty ? null : store.provider,
                  isDense: true,
                  underline: const SizedBox.shrink(),
                  dropdownColor: Palette.raised,
                  style: const TextStyle(color: Palette.muted, fontSize: 12),
                  items: store.providers
                      .map((p) => DropdownMenuItem(
                          value: p.name,
                          child: Text('${p.name} · ${p.modelId}')))
                      .toList(),
                  onChanged: store.sending
                      ? null
                      : (v) {
                          if (v != null) store.setProvider(v);
                        },
                ),
              ),
          ],
        ),
      ),
    );
  }
}
