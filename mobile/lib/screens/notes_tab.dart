import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';

import '../api/models.dart';
import '../main.dart';
import '../state/notes.dart';

class NotesTab extends StatefulWidget {
  const NotesTab({super.key});

  @override
  State<NotesTab> createState() => _NotesTabState();
}

class _NotesTabState extends State<NotesTab> {
  String _query = '';

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<NotesStore>().load();
    });
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<NotesStore>();
    final q = _query.trim().toLowerCase();
    final notes = q.isEmpty
        ? store.notes
        : store.notes
            .where((n) =>
                n.title.toLowerCase().contains(q) ||
                n.content.toLowerCase().contains(q))
            .toList();

    return Scaffold(
      backgroundColor: Palette.bg,
      floatingActionButton: FloatingActionButton(
        onPressed: () => _showEditor(context),
        child: const Icon(Icons.add),
      ),
      body: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
            child: TextField(
              onChanged: (v) => setState(() => _query = v),
              decoration: InputDecoration(
                hintText: 'Search notes…',
                prefixIcon: const Icon(Icons.search, size: 18, color: Palette.faint),
                isDense: true,
                contentPadding:
                    const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
                suffixIcon: _query.isNotEmpty
                    ? IconButton(
                        icon: const Icon(Icons.close, size: 16, color: Palette.faint),
                        onPressed: () => setState(() => _query = ''),
                      )
                    : null,
              ),
            ),
          ),
          Expanded(
            child: RefreshIndicator(
              onRefresh: store.load,
              child: store.loading && store.notes.isEmpty
                  ? const Center(child: CircularProgressIndicator())
                  : notes.isEmpty
                      ? ListView(
                          physics: const AlwaysScrollableScrollPhysics(),
                          children: [
                            Padding(
                              padding: const EdgeInsets.only(top: 120),
                              child: Center(
                                child: Text(
                                  store.notes.isEmpty
                                      ? 'No notes yet. Add one, or ask the AI.'
                                      : 'No notes match your search.',
                                  style: const TextStyle(color: Palette.faint),
                                ),
                              ),
                            ),
                          ],
                        )
                      : ListView.builder(
                          physics: const AlwaysScrollableScrollPhysics(),
                          padding: const EdgeInsets.only(bottom: 88),
                          itemCount: notes.length,
                          itemBuilder: (_, i) => _NoteTile(note: notes[i]),
                        ),
            ),
          ),
        ],
      ),
    );
  }
}

class _NoteTile extends StatelessWidget {
  const _NoteTile({required this.note});
  final Note note;

  @override
  Widget build(BuildContext context) {
    final snippet = note.content.replaceAll(RegExp(r'\s+'), ' ').trim();
    return ListTile(
      onTap: () => _showViewer(context, note),
      title: Text(note.title,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(
              color: Palette.fg, fontSize: 15, fontWeight: FontWeight.w500)),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(snippet,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(color: Palette.muted, fontSize: 12.5)),
          if (note.updatedAt != null)
            Padding(
              padding: const EdgeInsets.only(top: 2),
              child: Text(DateFormat('d MMM yyyy').format(note.updatedAt!),
                  style: const TextStyle(color: Palette.faint, fontSize: 11)),
            ),
        ],
      ),
    );
  }
}

void _showViewer(BuildContext context, Note note) {
  final store = context.read<NotesStore>();
  showModalBottomSheet(
    context: context,
    isScrollControlled: true,
    backgroundColor: Palette.surface,
    shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(16))),
    builder: (sheetCtx) => DraggableScrollableSheet(
      expand: false,
      initialChildSize: 0.6,
      maxChildSize: 0.92,
      builder: (_, scroll) => Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 16, 8, 0),
            child: Row(
              children: [
                Expanded(
                  child: Text(note.title,
                      style: const TextStyle(
                          color: Palette.fg,
                          fontSize: 16,
                          fontWeight: FontWeight.w600)),
                ),
                IconButton(
                  icon: const Icon(Icons.edit_outlined,
                      size: 19, color: Palette.muted),
                  onPressed: () {
                    Navigator.pop(sheetCtx);
                    _showEditor(context, note: note);
                  },
                ),
                IconButton(
                  icon: const Icon(Icons.delete_outline,
                      size: 19, color: Palette.danger),
                  onPressed: () async {
                    await store.remove(note);
                    if (sheetCtx.mounted) Navigator.pop(sheetCtx);
                  },
                ),
              ],
            ),
          ),
          Expanded(
            child: SingleChildScrollView(
              controller: scroll,
              padding: const EdgeInsets.fromLTRB(20, 8, 20, 24),
              child: Align(
                alignment: Alignment.topLeft,
                child: SelectableText(note.content,
                    style: const TextStyle(
                        color: Palette.fg, fontSize: 14, height: 1.5)),
              ),
            ),
          ),
        ],
      ),
    ),
  );
}

Future<void> _showEditor(BuildContext context, {Note? note}) async {
  final store = context.read<NotesStore>();
  final title = TextEditingController(text: note?.title ?? '');
  final content = TextEditingController(text: note?.content ?? '');

  await showModalBottomSheet(
    context: context,
    isScrollControlled: true,
    backgroundColor: Palette.surface,
    shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(16))),
    builder: (sheetCtx) => Padding(
      padding: EdgeInsets.only(
        left: 16,
        right: 16,
        top: 18,
        bottom: MediaQuery.of(sheetCtx).viewInsets.bottom + 18,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(note == null ? 'New note' : 'Edit note',
              style: const TextStyle(
                  color: Palette.fg, fontSize: 16, fontWeight: FontWeight.w600)),
          const SizedBox(height: 14),
          TextField(
            controller: title,
            autofocus: note == null,
            decoration: const InputDecoration(labelText: 'Title'),
          ),
          const SizedBox(height: 10),
          TextField(
            controller: content,
            maxLines: 8,
            minLines: 4,
            decoration: const InputDecoration(
                labelText: 'Write your note… (markdown supported)'),
          ),
          const SizedBox(height: 16),
          FilledButton(
            style: FilledButton.styleFrom(
              backgroundColor: Palette.accentBg,
              foregroundColor: Palette.accent,
              padding: const EdgeInsets.symmetric(vertical: 13),
            ),
            onPressed: () async {
              final t = title.text.trim();
              final c = content.text.trim();
              if (t.isEmpty || c.isEmpty) return;
              if (note == null) {
                await store.create(t, c);
              } else {
                await store.update(note, title: t, content: c);
              }
              if (sheetCtx.mounted) Navigator.pop(sheetCtx);
            },
            child: Text(note == null ? 'Add note' : 'Save'),
          ),
        ],
      ),
    ),
  );
}
