import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../api/models.dart';
import '../main.dart';
import '../state/activity.dart';
import '../state/auth.dart';
import '../state/chat.dart';
import '../state/notes.dart';
import '../state/shipments.dart';
import '../state/tasks.dart';
import 'activity_tab.dart';
import 'calendar_tab.dart';
import 'chat_tab.dart';
import 'email_tab.dart';
import 'notes_tab.dart';
import 'shipments_tab.dart';
import 'tasks_tab.dart';

/// Bottom-tab shell — the mobile counterpart of the desktop window workspace.
class ShellScreen extends StatefulWidget {
  const ShellScreen({super.key});

  @override
  State<ShellScreen> createState() => _ShellScreenState();
}

class _ShellScreenState extends State<ShellScreen> {
  int _tab = 0;

  static const _titles = [
    'Chat',
    'Email',
    'Calendar',
    'Tasks',
    'Notes',
    'Shipments',
    'Activity',
  ];
  static const _shipmentsTab = 5;
  static const _activityTab = 6;

  @override
  void initState() {
    super.initState();
    // One initial load so the approvals badge is right without visiting the tab.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<ActivityStore>().load();
    });
  }

  /// Jump from a job/approval to its conversation in the Chat tab.
  void _openSessionInChat(Session session) {
    context.read<ChatStore>().openSession(session);
    setState(() => _tab = 0);
  }

  @override
  Widget build(BuildContext context) {
    final pendingCount =
        context.watch<ActivityStore>().pendingCount;
    final pages = [
      const ChatTab(),
      const EmailTab(),
      const CalendarTab(),
      const TasksTab(),
      const NotesTab(),
      const ShipmentsTab(),
      ActivityTab(onOpenSession: _openSessionInChat),
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
        // Seven destinations don't fit with labels on a phone; show the label
        // only for the selected tab and let the icons carry the rest.
        labelBehavior: NavigationDestinationLabelBehavior.onlyShowSelected,
        selectedIndex: _tab,
        onDestinationSelected: (i) {
          setState(() => _tab = i);
          // Data changes server-side (chat tools, the web app, auto-sort) —
          // refresh a data tab whenever it's brought back into view. The
          // Activity tab polls while visible (job statuses flip server-side).
          if (i == 3) context.read<TasksStore>().load();
          if (i == 4) context.read<NotesStore>().load();
          if (i == _shipmentsTab) context.read<ShipmentsStore>().load();
          final activity = context.read<ActivityStore>();
          if (i == _activityTab) {
            activity.startPolling();
          } else {
            activity.stopPolling();
          }
        },
        destinations: [
          const NavigationDestination(icon: Icon(Icons.chat_bubble_outline), label: 'Chat'),
          const NavigationDestination(icon: Icon(Icons.mail_outline), label: 'Email'),
          const NavigationDestination(icon: Icon(Icons.calendar_today_outlined), label: 'Calendar'),
          const NavigationDestination(icon: Icon(Icons.check_circle_outline), label: 'Tasks'),
          const NavigationDestination(icon: Icon(Icons.description_outlined), label: 'Notes'),
          const NavigationDestination(icon: Icon(Icons.local_shipping_outlined), label: 'Shipments'),
          NavigationDestination(
            icon: Badge(
              isLabelVisible: pendingCount > 0,
              label: Text('$pendingCount'),
              child: const Icon(Icons.bolt_outlined),
            ),
            label: 'Activity',
          ),
        ],
      ),
    );
  }
}
