import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:provider/provider.dart' hide Provider;

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

class ChatTab extends StatefulWidget {
  const ChatTab({super.key});

  @override
  State<ChatTab> createState() => _ChatTabState();
}

class _ChatTabState extends State<ChatTab> {
  final _input = TextEditingController();
  final _scroll = ScrollController();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<ChatStore>().init();
    });
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
    if (text.isEmpty || store.sending) return;
    _input.clear();
    await store.send(text);
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
          _InputBar(input: _input, onSend: _send),
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
                child: Text(
                  '${message.toolNames.map(toolLabel).join(', ')}…',
                  style: const TextStyle(color: Color(0xFF7A9EC0), fontSize: 12.5),
                ),
              ),
            ],
          ),
        );
      case 'user':
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
            child: Text(message.displayText,
                style: const TextStyle(color: Palette.fg, fontSize: 14.5, height: 1.4)),
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
  const _InputBar({required this.input, required this.onSend});
  final TextEditingController input;
  final Future<void> Function() onSend;

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
