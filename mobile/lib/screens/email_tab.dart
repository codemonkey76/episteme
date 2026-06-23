import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart' hide Provider;

import '../api/models.dart';
import '../main.dart';
import '../state/email.dart';
import 'message_screen.dart';

class EmailTab extends StatefulWidget {
  const EmailTab({super.key});

  @override
  State<EmailTab> createState() => _EmailTabState();
}

class _EmailTabState extends State<EmailTab> {
  final _scroll = ScrollController();

  /// Multi-select (long-press to start): selected message ids.
  final _selected = <String>{};
  bool _deleting = false;

  void _toggleSelect(String id) {
    setState(() {
      if (!_selected.remove(id)) _selected.add(id);
    });
  }

  Future<void> _deleteSelected() async {
    if (_selected.isEmpty || _deleting) return;
    final count = _selected.length;
    final confirmed = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Palette.surface,
        title: Text('Delete $count message${count == 1 ? '' : 's'}?',
            style: const TextStyle(color: Palette.fg, fontSize: 16)),
        content: const Text('They move to Deleted Items (recoverable).',
            style: TextStyle(color: Palette.muted, fontSize: 13)),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: const Text('Cancel',
                style: TextStyle(color: Palette.muted)),
          ),
          TextButton(
            onPressed: () => Navigator.pop(ctx, true),
            child:
                const Text('Delete', style: TextStyle(color: Palette.danger)),
          ),
        ],
      ),
    );
    if (confirmed != true || !mounted) return;
    setState(() => _deleting = true);
    try {
      final deleted = await context
          .read<EmailStore>()
          .deleteMessages(_selected.toList());
      if (mounted) {
        _selected.clear();
        ScaffoldMessenger.of(context).showSnackBar(SnackBar(
            content: Text('Deleted $deleted message${deleted == 1 ? '' : 's'}'),
            duration: const Duration(seconds: 2)));
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('Delete failed: $e')));
      }
    } finally {
      if (mounted) setState(() => _deleting = false);
    }
  }

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<EmailStore>().init();
    });
    _scroll.addListener(() {
      if (_scroll.position.pixels >
          _scroll.position.maxScrollExtent - 400) {
        context.read<EmailStore>().loadMore();
      }
    });
  }

  void _showFolders() {
    final store = context.read<EmailStore>();
    showModalBottomSheet(
      context: context,
      backgroundColor: Palette.surface,
      shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(top: Radius.circular(16))),
      builder: (sheetCtx) => SafeArea(
        child: ListView(
          shrinkWrap: true,
          children: store.folders
              .map((f) => ListTile(
                    leading: Icon(Icons.folder_outlined,
                        size: 18,
                        color: f.id == store.folder?.id
                            ? Palette.accent
                            : Palette.faint),
                    title: Text(f.displayName,
                        style: TextStyle(
                            fontSize: 14,
                            color: f.id == store.folder?.id
                                ? Palette.fg
                                : Palette.muted)),
                    trailing: f.unread > 0
                        ? Text('${f.unread}',
                            style: const TextStyle(
                                color: Palette.accent, fontSize: 12))
                        : null,
                    onTap: () {
                      Navigator.pop(sheetCtx);
                      store.openFolder(f);
                    },
                  ))
              .toList(),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<EmailStore>();

    if (!store.checked) {
      return const Center(child: CircularProgressIndicator());
    }
    if (!store.connected) {
      return const Center(
        child: Padding(
          padding: EdgeInsets.all(32),
          child: Text(
            'No email account connected.\nConnect Microsoft 365 in the web app under Settings → Integrations.',
            textAlign: TextAlign.center,
            style: TextStyle(color: Palette.faint, fontSize: 13.5, height: 1.5),
          ),
        ),
      );
    }

    return Scaffold(
      backgroundColor: Palette.bg,
      body: Column(
        children: [
          // Folder bar — swaps to a selection bar while messages are selected.
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 2, 8, 0),
            child: _selected.isNotEmpty
                ? Row(
                    children: [
                      IconButton(
                        icon: const Icon(Icons.close,
                            size: 18, color: Palette.muted),
                        onPressed: () => setState(_selected.clear),
                      ),
                      Text('${_selected.length} selected',
                          style: const TextStyle(
                              color: Palette.fg, fontSize: 13.5)),
                      const Spacer(),
                      _deleting
                          ? const Padding(
                              padding: EdgeInsets.all(12),
                              child: SizedBox(
                                  width: 16,
                                  height: 16,
                                  child: CircularProgressIndicator(
                                      strokeWidth: 2)),
                            )
                          : IconButton(
                              icon: const Icon(Icons.delete_outline,
                                  size: 19, color: Palette.danger),
                              onPressed: _deleteSelected,
                            ),
                    ],
                  )
                : Row(
                    children: [
                      TextButton.icon(
                        onPressed: _showFolders,
                        icon: const Icon(Icons.folder_outlined,
                            size: 16, color: Palette.muted),
                        label: Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            Text(store.folder?.displayName ?? '',
                                style: const TextStyle(
                                    color: Palette.fg, fontSize: 13.5)),
                            if ((store.folder?.unread ?? 0) > 0)
                              Padding(
                                padding: const EdgeInsets.only(left: 6),
                                child: Text('${store.folder!.unread}',
                                    style: const TextStyle(
                                        color: Palette.accent, fontSize: 12)),
                              ),
                            const Icon(Icons.arrow_drop_down,
                                size: 18, color: Palette.faint),
                          ],
                        ),
                      ),
                      const Spacer(),
                      // Account switcher: hop between connected Microsoft 365
                      // connections. Hidden unless there's more than one.
                      if (store.accounts.length > 1)
                        PopupMenuButton<String>(
                          tooltip: 'Switch account',
                          color: Palette.surface,
                          onSelected: store.switchAccount,
                          itemBuilder: (_) => [
                            for (final a in store.accounts)
                              PopupMenuItem(
                                value: a.id,
                                child: Text(a.label,
                                    style: TextStyle(
                                        fontSize: 13.5,
                                        color: store.accountId == a.id
                                            ? Palette.accent
                                            : Palette.fg)),
                              ),
                          ],
                          child: Padding(
                            padding: const EdgeInsets.symmetric(horizontal: 4),
                            child: Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                const Icon(Icons.account_circle_outlined,
                                    size: 18, color: Palette.muted),
                                const SizedBox(width: 4),
                                ConstrainedBox(
                                  constraints:
                                      const BoxConstraints(maxWidth: 96),
                                  child: Text(
                                    store.currentAccountLabel,
                                    overflow: TextOverflow.ellipsis,
                                    softWrap: false,
                                    style: const TextStyle(
                                        color: Palette.fg, fontSize: 12.5),
                                  ),
                                ),
                                const Icon(Icons.arrow_drop_down,
                                    size: 18, color: Palette.faint),
                              ],
                            ),
                          ),
                        ),
                      // Mailbox switcher: own + any shared mailboxes added in
                      // web Settings. Hidden when there are none.
                      if (store.sharedMailboxes.isNotEmpty)
                        PopupMenuButton<String>(
                          tooltip: 'Switch mailbox',
                          color: Palette.surface,
                          icon: Icon(
                            Icons.inbox_outlined,
                            size: 18,
                            color: store.currentMailbox.isEmpty
                                ? Palette.muted
                                : Palette.accent,
                          ),
                          onSelected: store.switchMailbox,
                          itemBuilder: (_) => [
                            PopupMenuItem(
                              value: '',
                              child: Text('My mailbox',
                                  style: TextStyle(
                                      fontSize: 13.5,
                                      color: store.currentMailbox.isEmpty
                                          ? Palette.accent
                                          : Palette.fg)),
                            ),
                            for (final m in store.sharedMailboxes)
                              PopupMenuItem(
                                value: m.address,
                                child: Text(m.label,
                                    style: TextStyle(
                                        fontSize: 13.5,
                                        color:
                                            store.currentMailbox == m.address
                                                ? Palette.accent
                                                : Palette.fg)),
                              ),
                          ],
                        ),
                      IconButton(
                        icon: const Icon(Icons.refresh,
                            size: 18, color: Palette.muted),
                        onPressed: store.loading ? null : store.loadFolders,
                      ),
                    ],
                  ),
          ),
          // Commitment suggestion cards
          ...store.suggestions.map((s) => _SuggestionCard(suggestion: s)),
          Expanded(
            child: RefreshIndicator(
              onRefresh: store.loadFolders,
              child: store.loading && store.messages.isEmpty
                  ? const Center(child: CircularProgressIndicator())
                  : ListView.separated(
                      controller: _scroll,
                      physics: const AlwaysScrollableScrollPhysics(),
                      itemCount:
                          store.messages.length + (store.loadingMore ? 1 : 0),
                      separatorBuilder: (_, _) => const Divider(
                          height: 1, color: Color(0xFF161616)),
                      itemBuilder: (_, i) {
                        if (i >= store.messages.length) {
                          return const Padding(
                            padding: EdgeInsets.all(12),
                            child: Center(
                                child: SizedBox(
                                    width: 18,
                                    height: 18,
                                    child: CircularProgressIndicator(
                                        strokeWidth: 2))),
                          );
                        }
                        final m = store.messages[i];
                        return _MessageTile(
                          message: m,
                          selectionMode: _selected.isNotEmpty,
                          selected: _selected.contains(m.id),
                          onToggleSelect: () => _toggleSelect(m.id),
                        );
                      },
                    ),
            ),
          ),
        ],
      ),
    );
  }
}

class _MessageTile extends StatelessWidget {
  const _MessageTile({
    required this.message,
    required this.selectionMode,
    required this.selected,
    required this.onToggleSelect,
  });
  final MessageSummary message;
  final bool selectionMode;
  final bool selected;
  final VoidCallback onToggleSelect;

  String _when(DateTime? d) {
    if (d == null) return '';
    final now = DateTime.now();
    if (d.year == now.year && d.month == now.month && d.day == now.day) {
      return DateFormat('h:mm a').format(d);
    }
    return DateFormat('d MMM').format(d);
  }

  @override
  Widget build(BuildContext context) {
    final unread = !message.isRead;
    return ListTile(
      dense: true,
      selected: selected,
      selectedTileColor: const Color(0xFF14233A),
      onLongPress: onToggleSelect,
      leading: selectionMode
          ? Icon(
              selected ? Icons.check_circle : Icons.radio_button_unchecked,
              size: 19,
              color: selected ? Palette.accent : Palette.faint,
            )
          : null,
      onTap: selectionMode
          ? onToggleSelect
          : () => Navigator.of(context).push(MaterialPageRoute(
              builder: (_) => MessageScreen(summary: message))),
      title: Row(
        children: [
          if (message.flagStatus == 'flagged')
            const Padding(
              padding: EdgeInsets.only(right: 5),
              child: Icon(Icons.flag, size: 13, color: Color(0xFFDF6A6A)),
            ),
          Expanded(
            child: Text(
              message.from.display,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                fontSize: 13.5,
                color: unread ? Palette.fg : Palette.muted,
                fontWeight: unread ? FontWeight.w600 : FontWeight.w400,
              ),
            ),
          ),
          if (message.hasAttachments)
            const Padding(
              padding: EdgeInsets.only(left: 4),
              child: Icon(Icons.attach_file, size: 12, color: Palette.faint),
            ),
          const SizedBox(width: 6),
          Text(_when(message.received),
              style: TextStyle(
                  fontSize: 11.5,
                  color: unread ? Palette.accent : Palette.faint)),
        ],
      ),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(message.subject,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(
                  fontSize: 13,
                  color: unread ? const Color(0xFFB8C4D4) : Palette.muted,
                  fontWeight: unread ? FontWeight.w500 : FontWeight.w400)),
          Text(message.preview,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(fontSize: 12, color: Palette.faint)),
        ],
      ),
    );
  }
}

class _SuggestionCard extends StatelessWidget {
  const _SuggestionCard({required this.suggestion});
  final Suggestion suggestion;

  @override
  Widget build(BuildContext context) {
    final store = context.read<EmailStore>();
    final s = suggestion;
    return Container(
      margin: const EdgeInsets.fromLTRB(12, 4, 12, 4),
      padding: const EdgeInsets.all(10),
      decoration: BoxDecoration(
        color: const Color(0xFF10161A),
        border: Border.all(color: const Color(0xFF1E3A4A)),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Icon(s.kind == 'event' ? Icons.event : Icons.check_circle_outline,
                  size: 14, color: const Color(0xFF6AB8DF)),
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  'You committed to something — add ${s.kind == 'event' ? 'to calendar' : 'a task'}?',
                  style: const TextStyle(
                      color: Color(0xFF6AB8DF),
                      fontSize: 12.5,
                      fontWeight: FontWeight.w500),
                ),
              ),
            ],
          ),
          const SizedBox(height: 4),
          Text(s.title,
              style: const TextStyle(color: Palette.fg, fontSize: 13.5)),
          if (s.startAt != null)
            Text(DateFormat('EEE d MMM, h:mm a').format(s.startAt!),
                style: const TextStyle(color: Palette.faint, fontSize: 11.5)),
          const SizedBox(height: 6),
          Row(
            children: [
              FilledButton(
                style: FilledButton.styleFrom(
                  backgroundColor: const Color(0xFF1E3A2A),
                  foregroundColor: Palette.ok,
                  visualDensity: VisualDensity.compact,
                ),
                onPressed: () => store.acceptSuggestion(s),
                child: Text(s.kind == 'event' ? 'Add event' : 'Add task',
                    style: const TextStyle(fontSize: 12.5)),
              ),
              const SizedBox(width: 8),
              TextButton(
                onPressed: () => store.dismissSuggestion(s),
                child: const Text('Dismiss',
                    style: TextStyle(color: Palette.faint, fontSize: 12.5)),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
