/// Dart mirrors of the backend API types (see frontend/src/api.ts).
library;

import 'dart:convert';

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

class Session {
  Session({required this.id, required this.title, required this.updatedAt});

  final String id;
  final String title;
  final DateTime? updatedAt;

  factory Session.fromJson(Map<String, dynamic> j) => Session(
        id: j['id'] as String,
        title: j['title'] as String,
        updatedAt: DateTime.tryParse(j['updated_at'] as String? ?? '')?.toLocal(),
      );
}

/// One transcript entry. Roles: user | assistant | tool | tool_call.
class ChatMessage {
  ChatMessage({required this.id, required this.role, required this.content});

  final String id;
  final String role;
  String content;

  /// DB-stored content is JSON-encoded (a quoted string or a multimodal
  /// object); live-streamed content is plain text. Normalize for display.
  String get displayText {
    try {
      final v = jsonDecode(content);
      if (v is String) return v;
      if (v is Map && v['type'] == 'multimodal') return v['text'] as String? ?? '';
    } catch (_) {}
    return content;
  }

  /// For role `tool_call`: tool names — live chips hold a plain name, DB rows
  /// hold a JSON array of call objects.
  List<String> get toolNames {
    try {
      final v = jsonDecode(displayText);
      if (v is List) {
        return v
            .map((c) => (c is Map ? c['fn_name'] : null) as String? ?? '')
            .where((n) => n.isNotEmpty)
            .toList();
      }
    } catch (_) {}
    return [content];
  }

  factory ChatMessage.fromJson(Map<String, dynamic> j) => ChatMessage(
        id: j['id'] as String,
        role: j['role'] as String,
        content: j['content'] as String,
      );
}

class PendingApproval {
  PendingApproval({
    required this.id,
    required this.toolName,
    required this.toolArgs,
  });

  final String id;
  final String toolName;
  final String toolArgs;

  String get prettyArgs {
    try {
      return const JsonEncoder.withIndent('  ').convert(jsonDecode(toolArgs));
    } catch (_) {
      return toolArgs;
    }
  }
}

class Provider {
  Provider({required this.name, required this.modelId});

  final String name;
  final String modelId;

  factory Provider.fromJson(Map<String, dynamic> j) => Provider(
        name: j['name'] as String,
        modelId: j['model_id'] as String? ?? '',
      );
}
