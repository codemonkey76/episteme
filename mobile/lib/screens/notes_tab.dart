import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';

import '../api/models.dart';
import '../main.dart';
import '../state/notes.dart';
import 'note_screen.dart';

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
        onPressed: () => Navigator.of(context).push(MaterialPageRoute(
            fullscreenDialog: true, builder: (_) => const NoteEditorScreen())),
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
      onTap: () => Navigator.of(context).push(
          MaterialPageRoute(builder: (_) => NoteScreen(note: note))),
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

