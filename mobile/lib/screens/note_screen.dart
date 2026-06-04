import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart' hide Provider;

import '../api/models.dart';
import '../main.dart';
import '../state/notes.dart';

/// Full-page note viewer with rendered markdown.
class NoteScreen extends StatefulWidget {
  const NoteScreen({super.key, required this.note});
  final Note note;

  @override
  State<NoteScreen> createState() => _NoteScreenState();
}

class _NoteScreenState extends State<NoteScreen> {
  late Note note = widget.note;

  Future<void> _edit() async {
    final updated = await Navigator.of(context).push<Note>(MaterialPageRoute(
      fullscreenDialog: true,
      builder: (_) => NoteEditorScreen(note: note),
    ));
    if (updated != null && mounted) setState(() => note = updated);
  }

  Future<void> _delete() async {
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Palette.surface,
        title: const Text('Delete note?', style: TextStyle(fontSize: 16)),
        content: Text(note.title,
            maxLines: 2, overflow: TextOverflow.ellipsis),
        actions: [
          TextButton(
              onPressed: () => Navigator.pop(ctx, false),
              child: const Text('Cancel')),
          TextButton(
              onPressed: () => Navigator.pop(ctx, true),
              child:
                  const Text('Delete', style: TextStyle(color: Palette.danger))),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    await context.read<NotesStore>().remove(note);
    if (mounted) Navigator.pop(context);
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Palette.bg,
      appBar: AppBar(
        title: Text(note.title,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(fontSize: 16)),
        actions: [
          IconButton(
            icon: const Icon(Icons.edit_outlined, size: 19, color: Palette.muted),
            onPressed: _edit,
          ),
          IconButton(
            icon: const Icon(Icons.delete_outline,
                size: 19, color: Palette.danger),
            onPressed: _delete,
          ),
        ],
      ),
      body: Markdown(
        data: note.content,
        padding: const EdgeInsets.fromLTRB(16, 8, 16, 32),
        selectable: true,
        styleSheet: MarkdownStyleSheet(
          p: const TextStyle(color: Palette.fg, fontSize: 14.5, height: 1.5),
          code: const TextStyle(
              backgroundColor: Color(0xFF181818),
              color: Palette.fg,
              fontSize: 13),
          codeblockDecoration: BoxDecoration(
            color: const Color(0xFF0D0D0D),
            border: Border.all(color: const Color(0xFF222222)),
            borderRadius: BorderRadius.circular(6),
          ),
          listBullet: const TextStyle(color: Palette.fg, fontSize: 14.5),
          h1: const TextStyle(
              color: Color(0xFFE8E8E8), fontSize: 20, fontWeight: FontWeight.w600),
          h2: const TextStyle(
              color: Color(0xFFE8E8E8), fontSize: 17.5, fontWeight: FontWeight.w600),
          h3: const TextStyle(
              color: Color(0xFFE8E8E8), fontSize: 16, fontWeight: FontWeight.w600),
          blockquote: const TextStyle(color: Palette.muted),
          blockquoteDecoration: BoxDecoration(
            border: const Border(
                left: BorderSide(color: Palette.raised, width: 3)),
            color: const Color(0xFF131313),
            borderRadius: BorderRadius.circular(4),
          ),
          a: const TextStyle(color: Palette.accent),
          horizontalRuleDecoration: const BoxDecoration(
            border: Border(top: BorderSide(color: Color(0xFF222222))),
          ),
        ),
      ),
      bottomNavigationBar: note.updatedAt == null
          ? null
          : SafeArea(
              child: Padding(
                padding: const EdgeInsets.fromLTRB(16, 4, 16, 8),
                child: Text(
                  'Updated ${DateFormat('EEE d MMM yyyy, h:mm a').format(note.updatedAt!)}',
                  style: const TextStyle(color: Palette.faint, fontSize: 11.5),
                ),
              ),
            ),
    );
  }
}

/// Full-page editor for creating (`note == null`) or editing a note.
/// Pops with the saved Note.
class NoteEditorScreen extends StatefulWidget {
  const NoteEditorScreen({super.key, this.note});
  final Note? note;

  @override
  State<NoteEditorScreen> createState() => _NoteEditorScreenState();
}

class _NoteEditorScreenState extends State<NoteEditorScreen> {
  late final _title = TextEditingController(text: widget.note?.title ?? '');
  late final _content = TextEditingController(text: widget.note?.content ?? '');
  bool _saving = false;

  Future<void> _save() async {
    final t = _title.text.trim();
    final c = _content.text.trim();
    if (t.isEmpty || c.isEmpty || _saving) return;
    setState(() => _saving = true);
    final store = context.read<NotesStore>();
    try {
      if (widget.note == null) {
        await store.create(t, c);
      } else {
        await store.update(widget.note!, title: t, content: c);
      }
      final saved = store.notes.firstWhere(
        (n) => widget.note != null ? n.id == widget.note!.id : n.title == t,
        orElse: () => Note(
            id: widget.note?.id ?? '',
            title: t,
            content: c,
            updatedAt: DateTime.now()),
      );
      if (mounted) Navigator.pop(context, saved);
    } catch (e) {
      if (mounted) {
        setState(() => _saving = false);
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('Save failed: $e')));
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Palette.bg,
      appBar: AppBar(
        title: Text(widget.note == null ? 'New note' : 'Edit note',
            style: const TextStyle(fontSize: 16)),
        actions: [
          Padding(
            padding: const EdgeInsets.only(right: 8),
            child: FilledButton(
              style: FilledButton.styleFrom(
                backgroundColor: Palette.accentBg,
                foregroundColor: Palette.accent,
                visualDensity: VisualDensity.compact,
              ),
              onPressed: _saving ? null : _save,
              child: _saving
                  ? const SizedBox(
                      width: 13,
                      height: 13,
                      child: CircularProgressIndicator(strokeWidth: 2))
                  : const Text('Save'),
            ),
          ),
        ],
      ),
      body: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 16, 0),
        child: Column(
          children: [
            TextField(
              controller: _title,
              autofocus: widget.note == null,
              decoration: const InputDecoration(labelText: 'Title', isDense: true),
            ),
            const SizedBox(height: 10),
            Expanded(
              child: TextField(
                controller: _content,
                maxLines: null,
                expands: true,
                textAlignVertical: TextAlignVertical.top,
                decoration: const InputDecoration(
                  hintText: 'Write your note… (markdown supported)',
                  alignLabelWithHint: true,
                ),
              ),
            ),
            const SizedBox(height: 12),
          ],
        ),
      ),
    );
  }
}
