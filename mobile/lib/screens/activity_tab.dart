import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';

import '../api/models.dart';
import '../main.dart';
import '../state/activity.dart';
import 'report_screen.dart';

/// Jobs + approval queue + research reports — the mobile counterpart of the
/// web Jobs and Reports windows, sharing one tab.
class ActivityTab extends StatefulWidget {
  const ActivityTab({super.key, this.onOpenSession});

  /// Jump to the Chat tab with the given session loaded.
  final void Function(Session session)? onOpenSession;

  @override
  State<ActivityTab> createState() => _ActivityTabState();
}

class _ActivityTabState extends State<ActivityTab> {
  int _segment = 0; // 0 = Jobs, 1 = Reports

  static const _statusColor = {
    'running': Palette.accent,
    'needs_approval': Palette.warn,
    'done': Palette.ok,
    'failed': Palette.danger,
  };

  static String _fmtTime(DateTime? t) =>
      t == null ? '' : DateFormat('d MMM, h:mm a').format(t);

  void _openSession(String id, String title) {
    widget.onOpenSession
        ?.call(Session(id: id, title: title, updatedAt: null));
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<ActivityStore>();
    return Column(
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
          child: SegmentedButton<int>(
            segments: const [
              ButtonSegment(value: 0, label: Text('Jobs'), icon: Icon(Icons.bolt, size: 16)),
              ButtonSegment(value: 1, label: Text('Reports'), icon: Icon(Icons.article_outlined, size: 16)),
            ],
            selected: {_segment},
            onSelectionChanged: (s) => setState(() => _segment = s.first),
            showSelectedIcon: false,
            style: const ButtonStyle(visualDensity: VisualDensity.compact),
          ),
        ),
        if (store.error != null)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
            child: Text(store.error!,
                style: const TextStyle(color: Palette.danger, fontSize: 12)),
          ),
        Expanded(
          child: _segment == 0 ? _jobsView(store) : _reportsView(store),
        ),
      ],
    );
  }

  // ── Jobs ──────────────────────────────────────────────────────────────────

  Widget _jobsView(ActivityStore store) {
    if (store.loading && store.jobs.isEmpty && store.pending.isEmpty) {
      return const Center(child: CircularProgressIndicator());
    }
    return RefreshIndicator(
      onRefresh: store.load,
      child: ListView(
        physics: const AlwaysScrollableScrollPhysics(),
        padding: const EdgeInsets.fromLTRB(12, 4, 12, 24),
        children: [
          _sectionHeader('Needs your approval'),
          if (store.pending.isEmpty)
            const _EmptyNote(
                'Nothing waiting. Gated tools from background runs appear here.')
          else
            ...store.pending.map((a) => _approvalCard(store, a)),
          const SizedBox(height: 16),
          _sectionHeader('Recent runs'),
          if (store.jobs.isEmpty)
            const _EmptyNote(
                'No background or scheduled runs yet. Ask the assistant to '
                '"do something in the background".')
          else
            ...store.jobs.map(_jobRow),
        ],
      ),
    );
  }

  Widget _approvalCard(ActivityStore store, PendingApproval a) {
    return Container(
      margin: const EdgeInsets.only(bottom: 8),
      padding: const EdgeInsets.all(10),
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
              const Icon(Icons.warning_amber_rounded,
                  size: 15, color: Palette.warn),
              const SizedBox(width: 6),
              Expanded(
                child: Text(a.toolName,
                    style: const TextStyle(
                        color: Palette.warn,
                        fontSize: 13.5,
                        fontWeight: FontWeight.w600)),
              ),
              if (a.sessionTitle != null)
                GestureDetector(
                  onTap: () => _openSession(a.sessionId!, a.sessionTitle!),
                  child: Text(
                    a.sessionTitle!,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                        color: Color(0xFF8A7A50), fontSize: 11),
                  ),
                ),
            ],
          ),
          const SizedBox(height: 6),
          Container(
            width: double.infinity,
            constraints: const BoxConstraints(maxHeight: 130),
            padding: const EdgeInsets.all(8),
            decoration: BoxDecoration(
              color: const Color(0xFF12100A),
              borderRadius: BorderRadius.circular(6),
            ),
            child: SingleChildScrollView(
              child: Text(a.prettyArgs,
                  style: const TextStyle(
                      color: Color(0xFFA09070),
                      fontSize: 11.5,
                      fontFamily: 'monospace')),
            ),
          ),
          const SizedBox(height: 8),
          Row(
            children: [
              _decisionButton('Approve', Palette.ok, const Color(0xFF1E3A2A),
                  () => store.decide(a, true)),
              const SizedBox(width: 8),
              _decisionButton('Deny', Palette.danger, const Color(0xFF3A1E1E),
                  () => store.decide(a, false)),
              const Spacer(),
              Text(_fmtTime(a.createdAt),
                  style:
                      const TextStyle(color: Palette.faint, fontSize: 10.5)),
            ],
          ),
        ],
      ),
    );
  }

  Widget _decisionButton(
      String label, Color fg, Color bg, VoidCallback onTap) {
    return FilledButton(
      onPressed: onTap,
      style: FilledButton.styleFrom(
        backgroundColor: bg,
        foregroundColor: fg,
        visualDensity: VisualDensity.compact,
        padding: const EdgeInsets.symmetric(horizontal: 14),
        textStyle: const TextStyle(fontSize: 12.5),
      ),
      child: Text(label),
    );
  }

  Widget _jobRow(Job j) {
    final color = _statusColor[j.status] ?? Palette.muted;
    final kindIcon = switch (j.kind) {
      'scheduled' => Icons.schedule,
      'research' => Icons.travel_explore,
      _ => Icons.bolt,
    };
    return InkWell(
      onTap: () => _openSession(j.sessionId, j.name),
      borderRadius: BorderRadius.circular(8),
      child: Container(
        margin: const EdgeInsets.only(bottom: 6),
        padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
        decoration: BoxDecoration(
          color: Palette.surface,
          borderRadius: BorderRadius.circular(8),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(
              margin: const EdgeInsets.only(top: 2),
              padding:
                  const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
              decoration: BoxDecoration(
                border: Border.all(color: color.withValues(alpha: 0.5)),
                borderRadius: BorderRadius.circular(4),
              ),
              child: Text(
                j.status.replaceAll('_', ' '),
                style: TextStyle(
                    color: color,
                    fontSize: 9.5,
                    fontWeight: FontWeight.w600,
                    letterSpacing: 0.4),
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(
                    children: [
                      Expanded(
                        child: Text(j.name,
                            overflow: TextOverflow.ellipsis,
                            style: const TextStyle(
                                color: Palette.fg, fontSize: 13.5)),
                      ),
                      Icon(kindIcon, size: 13, color: Palette.faint),
                    ],
                  ),
                  if (j.summary?.isNotEmpty ?? false)
                    Text(j.summary!,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(
                            color: Palette.muted, fontSize: 11.5))
                  else if (j.error?.isNotEmpty ?? false)
                    Text(j.error!,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(
                            color: Palette.danger, fontSize: 11.5)),
                  Text(_fmtTime(j.updatedAt),
                      style: const TextStyle(
                          color: Palette.faint, fontSize: 10.5)),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  // ── Reports ───────────────────────────────────────────────────────────────

  Widget _reportsView(ActivityStore store) {
    return Column(
      children: [
        _ResearchLauncher(store: store),
        Expanded(
          child: store.loading && store.reports.isEmpty
              ? const Center(child: CircularProgressIndicator())
              : RefreshIndicator(
                  onRefresh: store.load,
                  child: store.reports.isEmpty
                      ? ListView(
                          physics: const AlwaysScrollableScrollPhysics(),
                          padding: const EdgeInsets.all(16),
                          children: const [
                            _EmptyNote(
                                'No reports yet. Research a topic above and '
                                'the result lands here.'),
                          ],
                        )
                      : ListView.builder(
                          physics: const AlwaysScrollableScrollPhysics(),
                          padding: const EdgeInsets.fromLTRB(12, 4, 12, 24),
                          itemCount: store.reports.length,
                          itemBuilder: (_, i) =>
                              _reportRow(store, store.reports[i]),
                        ),
                ),
        ),
      ],
    );
  }

  Widget _reportRow(ActivityStore store, Report r) {
    return Container(
      margin: const EdgeInsets.only(bottom: 6),
      decoration: BoxDecoration(
        color: Palette.surface,
        borderRadius: BorderRadius.circular(8),
      ),
      child: ListTile(
        dense: true,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
        title: Text(r.title,
            style: const TextStyle(color: Palette.fg, fontSize: 13.5)),
        subtitle: Text(
          r.createdAt == null
              ? ''
              : DateFormat('d MMM yyyy').format(r.createdAt!),
          style: const TextStyle(color: Palette.faint, fontSize: 11),
        ),
        trailing: IconButton(
          icon: const Icon(Icons.delete_outline, size: 18, color: Palette.faint),
          onPressed: () => _confirmDelete(store, r),
        ),
        onTap: () => Navigator.of(context).push(
          MaterialPageRoute(builder: (_) => ReportScreen(report: r)),
        ),
      ),
    );
  }

  Future<void> _confirmDelete(ActivityStore store, Report r) async {
    final yes = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Palette.surface,
        title: const Text('Delete report?',
            style: TextStyle(color: Palette.fg, fontSize: 16)),
        content: Text(r.title,
            style: const TextStyle(color: Palette.muted, fontSize: 13)),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: const Text('Cancel', style: TextStyle(color: Palette.muted)),
          ),
          TextButton(
            onPressed: () => Navigator.pop(ctx, true),
            child:
                const Text('Delete', style: TextStyle(color: Palette.danger)),
          ),
        ],
      ),
    );
    if (yes != true || !mounted) return;
    try {
      await store.deleteReport(r);
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('Delete failed: $e')));
    }
  }

  Widget _sectionHeader(String text) => Padding(
        padding: const EdgeInsets.only(top: 8, bottom: 6),
        child: Text(
          text.toUpperCase(),
          style: const TextStyle(
              color: Palette.faint,
              fontSize: 10.5,
              fontWeight: FontWeight.w600,
              letterSpacing: 0.8),
        ),
      );
}

class _EmptyNote extends StatelessWidget {
  const _EmptyNote(this.text);
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 4),
        child: Text(text,
            style: const TextStyle(color: Color(0xFF505050), fontSize: 12.5)),
      );
}

/// Topic field + depth picker + go — the "New research" launcher bar.
class _ResearchLauncher extends StatefulWidget {
  const _ResearchLauncher({required this.store});
  final ActivityStore store;

  @override
  State<_ResearchLauncher> createState() => _ResearchLauncherState();
}

class _ResearchLauncherState extends State<_ResearchLauncher> {
  final _topic = TextEditingController();
  String _depth = 'standard';
  bool _starting = false;

  @override
  void dispose() {
    _topic.dispose();
    super.dispose();
  }

  Future<void> _start() async {
    final t = _topic.text.trim();
    if (t.isEmpty || _starting) return;
    setState(() => _starting = true);
    try {
      await widget.store.startResearch(t, _depth);
      _topic.clear();
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(SnackBar(
            content: Text(
                'Researching "$t" — the report will appear here when ready.')));
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('Failed to start: $e')));
      }
    } finally {
      if (mounted) setState(() => _starting = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 4, 12, 8),
      child: Row(
        children: [
          Expanded(
            child: TextField(
              controller: _topic,
              style: const TextStyle(color: Palette.fg, fontSize: 13.5),
              decoration: const InputDecoration(
                hintText: 'Research a topic…',
                hintStyle: TextStyle(color: Color(0xFF404040), fontSize: 13),
                isDense: true,
                contentPadding:
                    EdgeInsets.symmetric(horizontal: 10, vertical: 10),
              ),
              textInputAction: TextInputAction.go,
              onSubmitted: (_) => _start(),
            ),
          ),
          const SizedBox(width: 8),
          DropdownButton<String>(
            value: _depth,
            dropdownColor: Palette.surface,
            style: const TextStyle(color: Palette.muted, fontSize: 12.5),
            underline: const SizedBox.shrink(),
            items: const [
              DropdownMenuItem(value: 'quick', child: Text('quick')),
              DropdownMenuItem(value: 'standard', child: Text('standard')),
              DropdownMenuItem(value: 'deep', child: Text('deep')),
            ],
            onChanged: (v) => setState(() => _depth = v ?? 'standard'),
          ),
          const SizedBox(width: 4),
          IconButton(
            onPressed: _starting ? null : _start,
            icon: _starting
                ? const SizedBox(
                    width: 16,
                    height: 16,
                    child: CircularProgressIndicator(strokeWidth: 2))
                : const Icon(Icons.travel_explore, color: Palette.accent),
            tooltip: 'Start research',
          ),
        ],
      ),
    );
  }
}
