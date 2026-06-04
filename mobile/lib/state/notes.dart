import 'package:flutter/foundation.dart';

import '../api/client.dart';
import '../api/models.dart';

class NotesStore extends ChangeNotifier {
  final _api = ApiClient.instance;

  List<Note> notes = [];
  bool loading = false;
  String? error;

  Future<void> load() async {
    loading = true;
    error = null;
    notifyListeners();
    try {
      final body = await _api.getJson('/notes', {'limit': '500'});
      notes = (body['notes'] as List)
          .map((n) => Note.fromJson(n as Map<String, dynamic>))
          .toList();
    } catch (e) {
      error = e.toString();
    } finally {
      loading = false;
      notifyListeners();
    }
  }

  Future<void> create(String title, String content) async {
    await _api.postJson('/notes', {'title': title, 'content': content});
    await load();
  }

  Future<void> update(Note note, {String? title, String? content}) async {
    await _api.putJson('/notes/${note.id}', {
      'title': ?title,
      'content': ?content,
    });
    await load();
  }

  Future<void> remove(Note note) async {
    await _api.delete('/notes/${note.id}');
    notes.removeWhere((n) => n.id == note.id);
    notifyListeners();
  }
}
