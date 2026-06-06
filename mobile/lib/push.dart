/// Push notifications via FCM — strictly optional infrastructure.
///
/// The app only gains push when `android/app/google-services.json` is present
/// at build time (the Gradle plugin is applied conditionally). Without it,
/// `Firebase.initializeApp` throws, we mark push unavailable, and every call
/// here is a silent no-op — login and the rest of the app are unaffected.
library;

import 'package:firebase_core/firebase_core.dart';
import 'package:firebase_messaging/firebase_messaging.dart';
import 'package:flutter/foundation.dart';

import 'api/client.dart';

class Push {
  static bool _available = false;

  /// Call once before runApp.
  static Future<void> init() async {
    try {
      await Firebase.initializeApp();
      _available = true;
    } catch (e) {
      debugPrint('push disabled (no Firebase config): $e');
      _available = false;
    }
  }

  /// Call after a session is established: asks notification permission,
  /// reports the device token, and keeps it fresh on rotation.
  static Future<void> register() async {
    if (!_available) return;
    try {
      final messaging = FirebaseMessaging.instance;
      await messaging.requestPermission();
      final token = await messaging.getToken();
      if (token != null) {
        await _report(token);
      }
      messaging.onTokenRefresh.listen((t) {
        _report(t);
      });
    } catch (e) {
      debugPrint('push registration failed: $e');
    }
  }

  static Future<void> _report(String token) async {
    try {
      await ApiClient.instance
          .postJson('/push/register', {'token': token, 'platform': 'android'});
    } catch (e) {
      debugPrint('push token report failed: $e');
    }
  }
}
