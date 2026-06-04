/// Dart mirrors of the backend API types (see frontend/src/api.ts).
library;

class Task {
  Task({
    required this.id,
    required this.title,
    required this.notes,
    required this.dueAt,
    required this.priority,
    required this.status,
  });

  final String id;
  final String title;
  final String? notes;
  final DateTime? dueAt;
  final String priority; // low | normal | high
  final String status; // open | done

  bool get isDone => status == 'done';
  bool get isOverdue =>
      !isDone && dueAt != null && dueAt!.isBefore(DateTime.now());

  factory Task.fromJson(Map<String, dynamic> j) => Task(
        id: j['id'] as String,
        title: j['title'] as String,
        notes: j['notes'] as String?,
        dueAt: j['due_at'] != null
            ? DateTime.tryParse(j['due_at'] as String)?.toLocal()
            : null,
        priority: j['priority'] as String? ?? 'normal',
        status: j['status'] as String? ?? 'open',
      );
}

class Note {
  Note({
    required this.id,
    required this.title,
    required this.content,
    required this.updatedAt,
  });

  final String id;
  final String title;
  final String content;
  final DateTime? updatedAt;

  factory Note.fromJson(Map<String, dynamic> j) => Note(
        id: j['id'] as String,
        title: j['title'] as String,
        content: j['content'] as String,
        updatedAt: DateTime.tryParse(j['updated_at'] as String? ?? '')?.toLocal(),
      );
}

class Suggestion {
  Suggestion({
    required this.id,
    required this.kind,
    required this.title,
    required this.startAt,
    required this.context,
  });

  final String id;
  final String kind; // task | event
  final String title;
  final DateTime? startAt;
  final String? context;

  factory Suggestion.fromJson(Map<String, dynamic> j) => Suggestion(
        id: j['id'] as String,
        kind: j['kind'] as String,
        title: j['title'] as String,
        startAt: j['start_at'] != null
            ? DateTime.tryParse(j['start_at'] as String)?.toLocal()
            : null,
        context: j['context'] as String?,
      );
}
