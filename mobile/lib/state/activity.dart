import 'dart:async';

import 'package:flutter/foundation.dart';

import '../api/client.dart';
import '../api/models.dart';

/// Jobs + global approval queue + research reports — the Activity tab.
/// Mirrors the web Jobs and Reports windows.
class ActivityStore extends ChangeNotifier {
  final _api = ApiClient.instance;

  List<Job> jobs = [];
  List<PendingApproval> pending = [];
  List<Report> reports = [];
  bool loading = false;
  String? error;

  Timer? _poll;

  /// Pending-approval count drives the badge on the Activity tab icon.
  int get pendingCount => pending.length;

  Future<void> load() async {
    loading = true;
    error = null;
    notifyListeners();
    try {
      await _fetch();
    } catch (e) {
      error = e.toString();
    } finally {
      loading = false;
      notifyListeners();
    }
  }

  Future<void> _fetch() async {
    final results = await Future.wait([
      _api.getJson('/jobs'),
      _api.getJson('/approvals/pending'),
      _api.getJson('/reports'),
    ]);
    jobs = (results[0]['jobs'] as List)
        .map((j) => Job.fromJson(j as Map<String, dynamic>))
        .toList();
    pending = (results[1]['pending_actions'] as List)
        .map((a) => PendingApproval.fromGlobalJson(a as Map<String, dynamic>))
        .toList();
    reports = (results[2]['reports'] as List)
        .map((r) => Report.fromJson(r as Map<String, dynamic>))
        .toList();
  }

  /// Poll while the tab is visible: approving the last pending action of a
  /// session resumes its job server-side, so statuses flip on their own.
  void startPolling() {
    _poll ??= Timer.periodic(const Duration(seconds: 5), (_) => _tick());
    load();
  }

  void stopPolling() {
    _poll?.cancel();
    _poll = null;
  }

  /// Background tick — no spinner; keep stale data on transient failures.
  Future<void> _tick() async {
    try {
      await _fetch();
      error = null;
      notifyListeners();
    } catch (_) {}
  }

  Future<void> decide(PendingApproval a, bool approved) async {
    // Optimistic removal; the follow-up load reconciles.
    pending.removeWhere((x) => x.id == a.id);
    notifyListeners();
    try {
      await _api.postJson(
          '/approvals/${a.id}/${approved ? 'approve' : 'reject'}', null);
    } catch (e) {
      error = e.toString();
    }
    await load();
  }

  Future<void> startResearch(String topic, String depth) async {
    await _api.postJson('/research', {'topic': topic, 'depth': depth});
    await load();
  }

  Future<String> reportHtml(Report r) => _api.getText('/reports/${r.id}/html');

  Future<void> deleteReport(Report r) async {
    await _api.delete('/reports/${r.id}');
    reports.removeWhere((x) => x.id == r.id);
    notifyListeners();
  }

  @override
  void dispose() {
    stopPolling();
    super.dispose();
  }
}
