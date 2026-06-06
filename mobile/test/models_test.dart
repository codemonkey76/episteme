import 'package:flutter_test/flutter_test.dart';
import 'package:episteme_mobile/api/models.dart';

void main() {
  test('Task parses API shape and derives overdue', () {
    final t = Task.fromJson({
      'id': 'x',
      'title': 'Buy milk',
      'notes': null,
      'due_at': '2020-01-01T00:00:00+00:00',
      'priority': 'high',
      'status': 'open',
    });
    expect(t.title, 'Buy milk');
    expect(t.isDone, false);
    expect(t.isOverdue, true);
    expect(t.dueAt, isNotNull);
  });

  test('Task without due date is never overdue', () {
    final t = Task.fromJson({
      'id': 'x',
      'title': 'Someday',
      'priority': 'low',
      'status': 'done',
    });
    expect(t.isDone, true);
    expect(t.isOverdue, false);
  });

  test('Note parses', () {
    final n = Note.fromJson({
      'id': 'n1',
      'title': 'Rack',
      'content': '# Layout',
      'updated_at': '2026-06-04T00:00:00+00:00',
    });
    expect(n.title, 'Rack');
    expect(n.updatedAt, isNotNull);
  });

  test('Job parses API shape with nullable summary/error', () {
    final j = Job.fromJson({
      'id': 'j1',
      'user_id': 'u1',
      'session_id': 's1',
      'kind': 'research',
      'name': 'Research: laptops',
      'provider': '',
      'status': 'needs_approval',
      'summary': null,
      'error': null,
      'meta': '{"topic":"laptops","depth":"standard"}',
      'created_at': '2026-06-07T01:00:00+00:00',
      'updated_at': '2026-06-07T01:05:00+00:00',
    });
    expect(j.kind, 'research');
    expect(j.status, 'needs_approval');
    expect(j.summary, isNull);
    expect(j.updatedAt, isNotNull);
  });

  test('Global pending approval parses and pretty-prints args', () {
    final a = PendingApproval.fromGlobalJson({
      'id': 'a1',
      'session_id': 's1',
      'session_title': '⏰ Morning briefing',
      'tool_name': 'send_email',
      'tool_args': '{"to":"bob@example.com"}',
      'created_at': '2026-06-07T01:00:00+00:00',
    });
    expect(a.toolName, 'send_email');
    expect(a.sessionTitle, '⏰ Morning briefing');
    expect(a.prettyArgs, contains('"to": "bob@example.com"'));
    // Chat-stream approvals still construct without session context.
    final chat = PendingApproval(id: 'a2', toolName: 't', toolArgs: 'not json');
    expect(chat.sessionTitle, isNull);
    expect(chat.prettyArgs, 'not json');
  });

  test('Report parses minimal metadata', () {
    final r = Report.fromJson({
      'id': 'r1',
      'title': 'Laptop comparison',
      'created_at': '2026-06-07T01:00:00+00:00',
    });
    expect(r.title, 'Laptop comparison');
    expect(r.createdAt, isNotNull);
  });
}

// (email text helpers)
