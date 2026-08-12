import 'dart:convert';

import 'package:flutter/foundation.dart';

import '../api/client.dart';
import '../api/models.dart';

/// Shipment tracking. `filter` mirrors the web window: 'active' (the default —
/// anything not delivered or cancelled), 'delivered', or 'all'.
class ShipmentsStore extends ChangeNotifier {
  final _api = ApiClient.instance;

  List<Shipment> shipments = [];
  String filter = 'active';
  bool loading = false;
  String? error;

  Future<void> load() async {
    loading = true;
    error = null;
    notifyListeners();
    try {
      final body = await _api.getJson('/shipments', {'status': filter});
      shipments = (body['shipments'] as List)
          .map((s) => Shipment.fromJson(s as Map<String, dynamic>))
          .toList();
    } catch (e) {
      error = e.toString();
    } finally {
      loading = false;
      notifyListeners();
    }
  }

  Future<void> setFilter(String value) async {
    if (filter == value) return;
    filter = value;
    await load();
  }

  Future<void> create(Map<String, Object?> fields) async {
    await _api.postJson('/shipments', fields);
    await load();
  }

  Future<void> update(Shipment shipment, Map<String, Object?> patch) async {
    await _api.putJson('/shipments/${shipment.id}', patch);
    await load();
  }

  Future<void> remove(Shipment shipment) async {
    await _api.delete('/shipments/${shipment.id}');
    shipments.removeWhere((s) => s.id == shipment.id);
    notifyListeners();
  }

  Future<void> addUpdate(Shipment shipment, String detail,
      {String? status}) async {
    await _api.postJson('/shipments/${shipment.id}/events', {
      'detail': detail,
      'status': ?status,
    });
    await load();
  }

  /// Attach a photo of what's on the way. Bytes go up base64-encoded in JSON,
  /// matching how the app already sends email attachments.
  Future<void> setPhoto(
      Shipment shipment, Uint8List bytes, String contentType) async {
    await _api.putJson('/shipments/${shipment.id}/photo', {
      'content_type': contentType,
      'content_bytes': base64Encode(bytes),
    });
    await load();
  }

  Future<void> removePhoto(Shipment shipment) async {
    await _api.delete('/shipments/${shipment.id}/photo');
    await load();
  }

  /// Authenticated URL for a shipment's photo. `updatedAt` busts the cache so a
  /// replaced photo actually shows the new one.
  String photoUrl(Shipment s) =>
      _api.url('/shipments/${s.id}/photo?v=${s.updatedAt?.toIso8601String() ?? ''}');

  Map<String, String> get photoHeaders => _api.authHeaders;

  /// The single shipment matching [id] after a reload, or null if it's gone.
  Shipment? byId(String id) {
    for (final s in shipments) {
      if (s.id == id) return s;
    }
    return null;
  }
}
