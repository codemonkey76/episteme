import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../main.dart';
import '../state/auth.dart';
import 'notes_tab.dart';
import 'tasks_tab.dart';

/// Bottom-tab shell — the mobile counterpart of the desktop window workspace.
class ShellScreen extends StatefulWidget {
  const ShellScreen({super.key});

  @override
  State<ShellScreen> createState() => _ShellScreenState();
}

class _ShellScreenState extends State<ShellScreen> {
  int _tab = 3; // Tasks first until Chat lands in phase 2.

  static const _titles = ['Chat', 'Email', 'Calendar', 'Tasks', 'Notes'];

  @override
  Widget build(BuildContext context) {
    final pages = [
      const _ComingSoon(label: 'Chat', phase: 'phase 2'),
      const _ComingSoon(label: 'Email', phase: 'phase 3'),
      const _ComingSoon(label: 'Calendar', phase: 'phase 3'),
      const TasksTab(),
      const NotesTab(),
    ];

    return Scaffold(
      appBar: AppBar(
        title: Text(_titles[_tab]),
        actions: [
          PopupMenuButton<String>(
            icon: const Icon(Icons.more_vert, color: Palette.muted),
            color: Palette.surface,
            onSelected: (v) {
              if (v == 'logout') context.read<AuthStore>().logout();
            },
            itemBuilder: (_) => const [
              PopupMenuItem(value: 'logout', child: Text('Sign out')),
            ],
          ),
        ],
      ),
      body: IndexedStack(index: _tab, children: pages),
      bottomNavigationBar: NavigationBar(
        selectedIndex: _tab,
        onDestinationSelected: (i) => setState(() => _tab = i),
        destinations: const [
          NavigationDestination(icon: Icon(Icons.chat_bubble_outline), label: 'Chat'),
          NavigationDestination(icon: Icon(Icons.mail_outline), label: 'Email'),
          NavigationDestination(icon: Icon(Icons.calendar_today_outlined), label: 'Calendar'),
          NavigationDestination(icon: Icon(Icons.check_circle_outline), label: 'Tasks'),
          NavigationDestination(icon: Icon(Icons.description_outlined), label: 'Notes'),
        ],
      ),
    );
  }
}

class _ComingSoon extends StatelessWidget {
  const _ComingSoon({required this.label, required this.phase});
  final String label;
  final String phase;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(Icons.construction, size: 36, color: Palette.faint.withValues(alpha: 0.5)),
          const SizedBox(height: 10),
          Text('$label is coming in $phase',
              style: const TextStyle(color: Palette.faint, fontSize: 14)),
        ],
      ),
    );
  }
}
