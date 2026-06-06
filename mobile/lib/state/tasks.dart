import 'package:flutter/foundation.dart';

import '../api/client.dart';
import '../api/models.dart';

class TasksStore extends ChangeNotifier {
  final _api = ApiClient.instance;

  List<Task> tasks = [];
  bool loading = false;
  String? error;

  /// Named to-do lists ("General" is implicit and not stored server-side).
  List<TodoList> lists = [];

  /// Active filter: '' = all lists, 'general' = the implicit General list,
  /// otherwise a named list's id.
  String currentList = '';

  Future<void> load() async {
    loading = true;
    error = null;
    notifyListeners();
    try {
      final body = await _api.getJson('/tasks', {
        'status': 'all',
        'limit': '500',
        if (currentList.isNotEmpty) 'list': currentList,
      });
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

  Future<void> loadLists() async {
    try {
      final body = await _api.getJson('/tasks/lists');
      lists = (body['lists'] as List)
          .map((l) => TodoList.fromJson(l as Map<String, dynamic>))
          .toList();
      notifyListeners();
    } catch (_) {
      // Lists are an enhancement; the flat view still works without them.
    }
  }

  Future<void> selectList(String list) async {
    currentList = list;
    await load();
  }

  Future<void> createList(String name) async {
    await _api.postJson('/tasks/lists', {'name': name});
    await loadLists();
  }

  Future<void> renameList(TodoList list, String name) async {
    await _api.putJson('/tasks/lists/${list.id}', {'name': name});
    await loadLists();
  }

  /// Deleting a list moves its tasks back to General (server-side).
  Future<void> deleteList(TodoList list) async {
    await _api.delete('/tasks/lists/${list.id}');
    if (currentList == list.id) currentList = '';
    await loadLists();
    await load();
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
      // New tasks land in the list being viewed (General/all → General).
      if (currentList.isNotEmpty && currentList != 'general')
        'list_id': currentList,
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
