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
}
