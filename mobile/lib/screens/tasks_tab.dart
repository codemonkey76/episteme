import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart';

import '../api/models.dart';
import '../main.dart';
import '../state/tasks.dart';

class TasksTab extends StatefulWidget {
  const TasksTab({super.key});

  @override
  State<TasksTab> createState() => _TasksTabState();
}

class _TasksTabState extends State<TasksTab> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<TasksStore>().load();
    });
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<TasksStore>();
    final open = store.tasks.where((t) => !t.isDone).toList();
    final done = store.tasks.where((t) => t.isDone).toList();

    return Scaffold(
      backgroundColor: Palette.bg,
      floatingActionButton: FloatingActionButton(
        onPressed: () => _showEditor(context),
        child: const Icon(Icons.add),
      ),
      body: RefreshIndicator(
        onRefresh: store.load,
        child: store.loading && store.tasks.isEmpty
            ? const Center(child: CircularProgressIndicator())
            : store.error != null && store.tasks.isEmpty
                ? _ErrorRetry(message: store.error!, onRetry: store.load)
                : ListView(
                    physics: const AlwaysScrollableScrollPhysics(),
                    padding: const EdgeInsets.only(bottom: 88),
                    children: [
                      if (store.tasks.isEmpty)
                        const Padding(
                          padding: EdgeInsets.only(top: 120),
                          child: Center(
                            child: Text('No tasks yet. Add one, or ask the AI.',
                                style: TextStyle(color: Palette.faint)),
                          ),
                        ),
                      ...open.map((t) => _TaskTile(task: t)),
                      if (done.isNotEmpty) ...[
                        const Padding(
                          padding: EdgeInsets.fromLTRB(16, 18, 16, 6),
                          child: Text('DONE',
                              style: TextStyle(
                                  color: Palette.faint,
                                  fontSize: 11,
                                  fontWeight: FontWeight.w600,
                                  letterSpacing: 1.2)),
                        ),
                        ...done.map((t) => _TaskTile(task: t)),
                      ],
                    ],
                  ),
      ),
    );
  }
}

class _TaskTile extends StatelessWidget {
  const _TaskTile({required this.task});
  final Task task;

  static const _prioColor = {
    'high': Color(0xFFFF8080),
    'normal': Palette.accent,
    'low': Color(0xFF9A9A9A),
  };

  @override
  Widget build(BuildContext context) {
    final store = context.read<TasksStore>();
    return Dismissible(
      key: ValueKey(task.id),
      direction: DismissDirection.endToStart,
      background: Container(
        color: const Color(0xFF3A1E1E),
        alignment: Alignment.centerRight,
        padding: const EdgeInsets.only(right: 20),
        child: const Icon(Icons.delete_outline, color: Palette.danger),
      ),
      onDismissed: (_) => store.remove(task),
      child: ListTile(
        onTap: () => _showEditor(context, task: task),
        leading: Checkbox(
          value: task.isDone,
          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(4)),
          side: const BorderSide(color: Palette.raised, width: 1.5),
          activeColor: Palette.ok,
          onChanged: (_) => store.toggleDone(task),
        ),
        title: Text(
          task.title,
          style: TextStyle(
            fontSize: 15,
            color: task.isDone ? Palette.faint : Palette.fg,
            decoration: task.isDone ? TextDecoration.lineThrough : null,
          ),
        ),
        subtitle: Row(
          children: [
            Text(task.priority.toUpperCase(),
                style: TextStyle(
                    fontSize: 10.5,
                    fontWeight: FontWeight.w600,
                    letterSpacing: 0.5,
                    color: _prioColor[task.priority] ?? Palette.muted)),
            if (task.dueAt != null) ...[
              const Text('  ·  ',
                  style: TextStyle(color: Palette.faint, fontSize: 11)),
              Text(
                'due ${DateFormat('EEE d MMM, h:mm a').format(task.dueAt!)}',
                style: TextStyle(
                    fontSize: 11.5,
                    color: task.isOverdue ? const Color(0xFFFF7070) : Palette.faint),
              ),
            ],
          ],
        ),
      ),
    );
  }
}

class _ErrorRetry extends StatelessWidget {
  const _ErrorRetry({required this.message, required this.onRetry});
  final String message;
  final Future<void> Function() onRetry;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(message,
              textAlign: TextAlign.center,
              style: const TextStyle(color: Palette.danger, fontSize: 13)),
          const SizedBox(height: 10),
          TextButton(onPressed: onRetry, child: const Text('Retry')),
        ],
      ),
    );
  }
}

/// Bottom-sheet editor for creating or editing a task.
Future<void> _showEditor(BuildContext context, {Task? task}) async {
  final store = context.read<TasksStore>();
  final title = TextEditingController(text: task?.title ?? '');
  final notes = TextEditingController(text: task?.notes ?? '');
  var priority = task?.priority ?? 'normal';
  DateTime? due = task?.dueAt;
  var dueCleared = false;

  await showModalBottomSheet(
    context: context,
    isScrollControlled: true,
    backgroundColor: Palette.surface,
    shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(16))),
    builder: (sheetCtx) => StatefulBuilder(
      builder: (ctx, setSheet) => Padding(
        padding: EdgeInsets.only(
          left: 16,
          right: 16,
          top: 18,
          bottom: MediaQuery.of(ctx).viewInsets.bottom + 18,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(task == null ? 'New task' : 'Edit task',
                style: const TextStyle(
                    color: Palette.fg, fontSize: 16, fontWeight: FontWeight.w600)),
            const SizedBox(height: 14),
            TextField(
              controller: title,
              autofocus: task == null,
              decoration: const InputDecoration(labelText: 'What needs doing?'),
            ),
            const SizedBox(height: 10),
            TextField(
              controller: notes,
              maxLines: 3,
              minLines: 2,
              decoration: const InputDecoration(labelText: 'Notes (optional)'),
            ),
            const SizedBox(height: 10),
            Row(
              children: [
                Expanded(
                  child: OutlinedButton.icon(
                    icon: const Icon(Icons.schedule, size: 16),
                    label: Text(
                      due == null
                          ? 'Due time'
                          : DateFormat('EEE d MMM, h:mm a').format(due!),
                      style: const TextStyle(fontSize: 12.5),
                    ),
                    onPressed: () async {
                      final now = DateTime.now();
                      final date = await showDatePicker(
                        context: ctx,
                        initialDate: due ?? now,
                        firstDate: now.subtract(const Duration(days: 1)),
                        lastDate: now.add(const Duration(days: 365 * 3)),
                      );
                      if (date == null || !ctx.mounted) return;
                      final time = await showTimePicker(
                        context: ctx,
                        initialTime: TimeOfDay.fromDateTime(due ?? now),
                      );
                      if (time == null) return;
                      setSheet(() {
                        due = DateTime(date.year, date.month, date.day,
                            time.hour, time.minute);
                        dueCleared = false;
                      });
                    },
                  ),
                ),
                if (due != null)
                  IconButton(
                    icon: const Icon(Icons.close, size: 18, color: Palette.muted),
                    onPressed: () => setSheet(() {
                      due = null;
                      dueCleared = true;
                    }),
                  ),
                const SizedBox(width: 8),
                DropdownButton<String>(
                  value: priority,
                  dropdownColor: Palette.raised,
                  items: const [
                    DropdownMenuItem(value: 'low', child: Text('low')),
                    DropdownMenuItem(value: 'normal', child: Text('normal')),
                    DropdownMenuItem(value: 'high', child: Text('high')),
                  ],
                  onChanged: (v) => setSheet(() => priority = v ?? 'normal'),
                ),
              ],
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
                if (t.isEmpty) return;
                if (task == null) {
                  await store.create(
                    title: t,
                    notes: notes.text.trim(),
                    dueAt: due,
                    priority: priority,
                  );
                } else {
                  await store.update(
                    task,
                    title: t,
                    notes: notes.text.trim(),
                    dueAt: due,
                    clearDue: dueCleared && due == null,
                    priority: priority,
                  );
                }
                if (sheetCtx.mounted) Navigator.pop(sheetCtx);
              },
              child: Text(task == null ? 'Add task' : 'Save'),
            ),
          ],
        ),
      ),
    ),
  );
}
