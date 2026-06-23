/// Dart mirrors of the backend API types (see frontend/src/api.ts).
library;

import 'dart:convert';
import 'dart:typed_data';

class Task {
  Task({
    required this.id,
    required this.title,
    required this.notes,
    required this.dueAt,
    required this.priority,
    required this.status,
    required this.listId,
  });

  final String id;
  final String title;
  final String? notes;
  final DateTime? dueAt;
  final String priority; // low | normal | high
  final String status; // open | done
  /// To-do list this task belongs to; null = the implicit "General" list.
  final String? listId;

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
        listId: j['list_id'] as String?,
      );
}

class TodoList {
  TodoList({required this.id, required this.name});
  final String id;
  final String name;

  factory TodoList.fromJson(Map<String, dynamic> j) => TodoList(
        id: j['id'] as String,
        name: j['name'] as String,
      );
}

class SharedMailbox {
  SharedMailbox({required this.address, required this.name});
  final String address;
  final String? name;

  String get label => (name?.isNotEmpty ?? false) ? name! : address;

  factory SharedMailbox.fromJson(Map<String, dynamic> j) => SharedMailbox(
        address: j['address'] as String,
        name: j['name'] as String?,
      );
}

/// A connected Microsoft 365 instance. `id` is the integration id threaded as
/// `account=` into every Graph-backed call; `email` is its own mailbox address.
class EmailAccount {
  EmailAccount({required this.id, required this.name, required this.email});
  final String id;
  final String name;
  final String email;

  String get label => name.isNotEmpty ? name : email;

  factory EmailAccount.fromJson(Map<String, dynamic> j) => EmailAccount(
        id: j['id'] as String,
        name: (j['name'] as String?) ?? '',
        email: (j['account'] as String?) ?? '',
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

  /// Decoded image bytes of a multimodal user message (empty otherwise).
  List<Uint8List> get displayImages {
    try {
      final v = jsonDecode(content);
      if (v is Map && v['type'] == 'multimodal' && v['images'] is List) {
        return (v['images'] as List)
            .map((i) => (i is Map ? i['b64'] : null) as String?)
            .whereType<String>()
            .map(base64Decode)
            .toList();
      }
    } catch (_) {}
    return const [];
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
    this.sessionId,
    this.sessionTitle,
    this.createdAt,
  });

  final String id;
  final String toolName;
  final String toolArgs;
  // Set only for the global queue (/approvals/pending), not chat approvals.
  final String? sessionId;
  final String? sessionTitle;
  final DateTime? createdAt;

  /// One row of GET /approvals/pending — the global approval queue.
  factory PendingApproval.fromGlobalJson(Map<String, dynamic> j) =>
      PendingApproval(
        id: j['id'] as String,
        toolName: j['tool_name'] as String,
        toolArgs: j['tool_args'] as String? ?? '',
        sessionId: j['session_id'] as String?,
        sessionTitle: j['session_title'] as String?,
        createdAt: j['created_at'] != null
            ? DateTime.tryParse(j['created_at'] as String)?.toLocal()
            : null,
      );

  String get prettyArgs {
    try {
      return const JsonEncoder.withIndent('  ').convert(jsonDecode(toolArgs));
    } catch (_) {
      return toolArgs;
    }
  }
}

class Job {
  Job({
    required this.id,
    required this.sessionId,
    required this.kind,
    required this.name,
    required this.status,
    required this.summary,
    required this.error,
    required this.updatedAt,
  });

  final String id;
  final String sessionId;
  final String kind; // background | scheduled | research
  final String name;
  final String status; // running | needs_approval | done | failed
  final String? summary;
  final String? error;
  final DateTime? updatedAt;

  factory Job.fromJson(Map<String, dynamic> j) => Job(
        id: j['id'] as String,
        sessionId: j['session_id'] as String,
        kind: j['kind'] as String? ?? 'background',
        name: j['name'] as String? ?? '',
        status: j['status'] as String? ?? 'running',
        summary: j['summary'] as String?,
        error: j['error'] as String?,
        updatedAt: j['updated_at'] != null
            ? DateTime.tryParse(j['updated_at'] as String)?.toLocal()
            : null,
      );
}

class Report {
  Report({
    required this.id,
    required this.title,
    required this.createdAt,
    this.shareToken,
  });

  final String id;
  final String title;
  final DateTime? createdAt;

  /// Public share token; non-null = anyone with the link can view it.
  /// Mutable so share/revoke updates the in-place list without a refetch.
  String? shareToken;

  factory Report.fromJson(Map<String, dynamic> j) => Report(
        id: j['id'] as String,
        title: j['title'] as String? ?? '',
        createdAt: j['created_at'] != null
            ? DateTime.tryParse(j['created_at'] as String)?.toLocal()
            : null,
        shareToken: j['share_token'] as String?,
      );
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

class EmailAddress {
  EmailAddress({required this.name, required this.address});
  final String name;
  final String address;

  factory EmailAddress.fromJson(Map<String, dynamic>? j) => EmailAddress(
        name: (j?['name'] as String?) ?? '',
        address: (j?['address'] as String?) ?? '',
      );

  String get display => name.isNotEmpty ? name : address;
}

class MailFolder {
  MailFolder({required this.id, required this.displayName, required this.unread});
  final String id;
  final String displayName;
  int unread;

  factory MailFolder.fromJson(Map<String, dynamic> j) => MailFolder(
        id: j['id'] as String,
        displayName: j['displayName'] as String,
        unread: (j['unreadItemCount'] as num?)?.toInt() ?? 0,
      );
}

class MessageSummary {
  MessageSummary({
    required this.id,
    required this.subject,
    required this.from,
    required this.preview,
    required this.received,
    required this.isRead,
    required this.hasAttachments,
    required this.flagStatus,
  });

  final String id;
  final String subject;
  final EmailAddress from;
  final String preview;
  final DateTime? received;
  bool isRead;
  final bool hasAttachments;
  final String flagStatus; // notFlagged | flagged | complete

  factory MessageSummary.fromJson(Map<String, dynamic> j) => MessageSummary(
        id: j['id'] as String,
        subject: (j['subject'] as String?) ?? '(no subject)',
        from: EmailAddress.fromJson(
            (j['from'] as Map<String, dynamic>?)?['emailAddress']
                as Map<String, dynamic>?),
        preview: (j['bodyPreview'] as String?) ?? '',
        received: DateTime.tryParse(j['receivedDateTime'] as String? ?? '')
            ?.toLocal(),
        isRead: j['isRead'] as bool? ?? true,
        hasAttachments: j['hasAttachments'] as bool? ?? false,
        flagStatus:
            ((j['flag'] as Map<String, dynamic>?)?['flagStatus'] as String?) ??
                'notFlagged',
      );
}

class MessageDetail {
  MessageDetail({
    required this.summary,
    required this.to,
    required this.cc,
    required this.bodyIsHtml,
    required this.body,
  });

  final MessageSummary summary;
  final List<EmailAddress> to;
  final List<EmailAddress> cc;
  final bool bodyIsHtml;
  final String body;

  factory MessageDetail.fromJson(Map<String, dynamic> j) {
    List<EmailAddress> addrs(String key) => ((j[key] as List?) ?? [])
        .map((r) => EmailAddress.fromJson(
            (r as Map<String, dynamic>)['emailAddress'] as Map<String, dynamic>?))
        .toList();
    final body = j['body'] as Map<String, dynamic>? ?? {};
    return MessageDetail(
      summary: MessageSummary.fromJson(j),
      to: addrs('toRecipients'),
      cc: addrs('ccRecipients'),
      bodyIsHtml:
          (body['contentType'] as String? ?? '').toLowerCase() == 'html',
      body: (body['content'] as String?) ?? '',
    );
  }
}

class Attachment {
  Attachment({
    required this.id,
    required this.name,
    required this.contentType,
    required this.isInline,
    this.contentId,
  });

  final String id;
  final String name;
  final String contentType;
  final bool isInline;
  final String? contentId;

  factory Attachment.fromJson(Map<String, dynamic> j) => Attachment(
        id: j['id'] as String,
        name: (j['name'] as String?) ?? '',
        contentType: (j['contentType'] as String?) ?? '',
        isInline: j['isInline'] as bool? ?? false,
        contentId: j['contentId'] as String?,
      );
}

class CalendarEvent {
  CalendarEvent({
    required this.id,
    required this.subject,
    required this.start,
    required this.end,
    required this.location,
    required this.isAllDay,
  });

  final String id;
  final String subject;
  final DateTime? start;
  final DateTime? end;
  final String location;
  final bool isAllDay;

  factory CalendarEvent.fromJson(Map<String, dynamic> j) => CalendarEvent(
        id: j['id'] as String,
        subject: (j['subject'] as String?) ?? '(no subject)',
        start: DateTime.tryParse(j['start'] as String? ?? '')?.toLocal(),
        end: DateTime.tryParse(j['end'] as String? ?? '')?.toLocal(),
        location: (j['location'] as String?) ?? '',
        isAllDay: j['is_all_day'] as bool? ?? false,
      );
}
