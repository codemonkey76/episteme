import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../main.dart';
import '../state/auth.dart';
import '../state/notes.dart';
import '../state/tasks.dart';
import 'calendar_tab.dart';
import 'chat_tab.dart';
import 'email_tab.dart';
import 'notes_tab.dart';
import 'tasks_tab.dart';

/// Bottom-tab shell — the mobile counterpart of the desktop window workspace.
class ShellScreen extends StatefulWidget {
  const ShellScreen({super.key});

  @override
  State<ShellScreen> createState() => _ShellScreenState();
}

class _ShellScreenState extends State<ShellScreen> {
  int _tab = 0;

  static const _titles = ['Chat', 'Email', 'Calendar', 'Tasks', 'Notes'];

  @override
  Widget build(BuildContext context) {
    final pages = [
      const ChatTab(),
      const EmailTab(),
      const CalendarTab(),
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
        onDestinationSelected: (i) {
          setState(() => _tab = i);
          // Data changes server-side (chat tools, the web app, auto-sort) —
          // refresh a data tab whenever it's brought back into view.
          if (i == 3) context.read<TasksStore>().load();
          if (i == 4) context.read<NotesStore>().load();
        },
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

