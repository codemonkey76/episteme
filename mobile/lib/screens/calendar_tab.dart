import 'package:flutter/material.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart' hide Provider;

import '../api/client.dart';
import '../api/models.dart';
import '../main.dart';
import '../state/email.dart';

/// Agenda view: the next 30 days of events grouped by day.
class CalendarTab extends StatefulWidget {
  const CalendarTab({super.key});

  @override
  State<CalendarTab> createState() => CalendarTabState();
}

class CalendarTabState extends State<CalendarTab> {
  List<CalendarEvent> events = [];
  bool loading = false;
  String? error;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => load());
  }

  Future<void> load() async {
    setState(() {
      loading = true;
      error = null;
    });
    try {
      final now = DateTime.now();
      final start = DateTime(now.year, now.month, now.day).toUtc();
      final end = start.add(const Duration(days: 30));
      final res = await ApiClient.instance.getJson('/calendar/events', {
        'start': start.toIso8601String(),
        'end': end.toIso8601String(),
      });
      events = (res['events'] as List)
          .map((e) => CalendarEvent.fromJson(e as Map<String, dynamic>))
          .toList()
        ..sort((a, b) => (a.start ?? DateTime(0)).compareTo(b.start ?? DateTime(0)));
    } catch (e) {
      error = e.toString();
    } finally {
      if (mounted) setState(() => loading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    // The email connection gates calendar too (both are Microsoft 365).
    final email = context.watch<EmailStore>();
    if (email.checked && !email.connected) {
      return const Center(
        child: Padding(
          padding: EdgeInsets.all(32),
          child: Text(
            'Calendar needs the Microsoft 365 connection.\nConnect it in the web app under Settings → Integrations.',
            textAlign: TextAlign.center,
            style: TextStyle(color: Palette.faint, fontSize: 13.5, height: 1.5),
          ),
        ),
      );
    }

    // Group events by day.
    final groups = <DateTime, List<CalendarEvent>>{};
    for (final e in events) {
      final s = e.start;
      if (s == null) continue;
      final day = DateTime(s.year, s.month, s.day);
      groups.putIfAbsent(day, () => []).add(e);
    }
    final days = groups.keys.toList()..sort();

    return RefreshIndicator(
      onRefresh: load,
      child: loading && events.isEmpty
          ? const Center(child: CircularProgressIndicator())
          : error != null && events.isEmpty
              ? ListView(
                  physics: const AlwaysScrollableScrollPhysics(),
                  children: [
                    Padding(
                      padding: const EdgeInsets.all(32),
                      child: Text(error!,
                          textAlign: TextAlign.center,
                          style: const TextStyle(
                              color: Palette.danger, fontSize: 13)),
                    ),
                  ],
                )
              : days.isEmpty
                  ? ListView(
                      physics: const AlwaysScrollableScrollPhysics(),
                      children: const [
                        Padding(
                          padding: EdgeInsets.only(top: 120),
                          child: Center(
                            child: Text('Nothing scheduled in the next 30 days.',
                                style: TextStyle(color: Palette.faint)),
                          ),
                        ),
                      ],
                    )
                  : ListView.builder(
                      physics: const AlwaysScrollableScrollPhysics(),
                      padding: const EdgeInsets.only(bottom: 24),
                      itemCount: days.length,
                      itemBuilder: (_, i) {
                        final day = days[i];
                        final dayEvents = groups[day]!;
                        final isToday = day ==
                            DateTime(DateTime.now().year,
                                DateTime.now().month, DateTime.now().day);
                        return Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Padding(
                              padding:
                                  const EdgeInsets.fromLTRB(16, 16, 16, 6),
                              child: Text(
                                isToday
                                    ? 'Today — ${DateFormat('EEEE d MMMM').format(day)}'
                                    : DateFormat('EEEE d MMMM').format(day),
                                style: TextStyle(
                                  fontSize: 12,
                                  fontWeight: FontWeight.w600,
                                  letterSpacing: 0.6,
                                  color: isToday
                                      ? Palette.accent
                                      : Palette.faint,
                                ),
                              ),
                            ),
                            ...dayEvents.map((e) => ListTile(
                                  dense: true,
                                  leading: SizedBox(
                                    width: 64,
                                    child: Text(
                                      e.isAllDay
                                          ? 'all day'
                                          : (e.start != null
                                              ? DateFormat('h:mm a')
                                                  .format(e.start!)
                                              : ''),
                                      style: const TextStyle(
                                          color: Palette.accent,
                                          fontSize: 12.5),
                                    ),
                                  ),
                                  title: Text(e.subject,
                                      style: const TextStyle(
                                          color: Palette.fg, fontSize: 14)),
                                  subtitle: e.location.isNotEmpty
                                      ? Text(e.location,
                                          maxLines: 1,
                                          overflow: TextOverflow.ellipsis,
                                          style: const TextStyle(
                                              color: Palette.faint,
                                              fontSize: 12))
                                      : null,
                                )),
                          ],
                        );
                      },
                    ),
    );
  }
}
