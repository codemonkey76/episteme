import 'package:flutter/foundation.dart';

import '../api/client.dart';
import '../api/models.dart';

class TasksStore extends ChangeNotifier {
  final _api = ApiClient.instance;

  List<Task> tasks = [];
  bool loading = false;
  String? error;

  Future<void> load() async {
    loading = true;
    error = null;
    notifyListeners();
    try {
      final body = await _api.getJson('/tasks', {'status': 'all', 'limit': '500'});
      tasks = (body['tasks'] as List)
          .map((t) => Task.fromJson(t as Map<String, dynamic>))
          .toList();
    } catch (e) {
      error = e.toString();
    } finally {
      loading = false;
      notifyListeners();
    }
  }

  Future<void> create({
    required String title,
    String? notes,
    DateTime? dueAt,
    String priority = 'normal',
  }) async {
    await _api.postJson('/tasks', {
      'title': title,
      if (notes != null && notes.isNotEmpty) 'notes': notes,
      if (dueAt != null) 'due_at': dueAt.toUtc().toIso8601String(),
      'priority': priority,
    });
    await load();
  }

  Future<void> update(
    Task task, {
    String? title,
    String? notes,
    DateTime? dueAt,
    bool clearDue = false,
    String? priority,
    String? status,
  }) async {
    await _api.putJson('/tasks/${task.id}', {
      'title': ?title,
      if (notes != null) 'notes': notes.isEmpty ? null : notes,
      if (clearDue)
        'due_at': null
      else if (dueAt != null)
        'due_at': dueAt.toUtc().toIso8601String(),
      'priority': ?priority,
      'status': ?status,
    });
    await load();
  }

  Future<void> toggleDone(Task task) =>
      update(task, status: task.isDone ? 'open' : 'done');

  Future<void> remove(Task task) async {
    await _api.delete('/tasks/${task.id}');
    tasks.removeWhere((t) => t.id == task.id);
    notifyListeners();
  }
}
