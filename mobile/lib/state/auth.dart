import 'package:flutter/foundation.dart';

import '../api/client.dart';
import '../push.dart';

enum AuthState { unknown, loggedOut, loggedIn }

class AuthStore extends ChangeNotifier {
  AuthState state = AuthState.unknown;
  String? error;

  final _api = ApiClient.instance;

  /// App start: restore stored server/cookie and verify the session.
  Future<void> init() async {
    final restored = await _api.restore();
    if (!restored) {
      state = AuthState.loggedOut;
      notifyListeners();
      return;
    }
    try {
      state = await _api.authStatus() ? AuthState.loggedIn : AuthState.loggedOut;
    } catch (_) {
      // Server unreachable — keep the session and let the user in; requests
      // will surface errors per-screen.
      state = AuthState.loggedIn;
    }
    if (state == AuthState.loggedIn) {
      Push.register(); // fire-and-forget; no-op without Firebase config
    }
    notifyListeners();
  }

  /// Password was accepted but the account wants a TOTP/recovery code too —
  /// the login screen shows the code field and resubmits.
  bool totpRequired = false;

  Future<bool> login(String server, String username, String password,
      {String? code}) async {
    error = null;
    try {
      await _api.setServer(server);
      final result = await _api.login(username, password, code: code);
      if (result == LoginResult.totpRequired) {
        totpRequired = true;
        notifyListeners();
        return false;
      }
      totpRequired = false;
      state = AuthState.loggedIn;
      Push.register(); // fire-and-forget; no-op without Firebase config
      notifyListeners();
      return true;
    } catch (e) {
      error = e.toString();
      notifyListeners();
      return false;
    }
  }

  Future<void> logout() async {
    await _api.logout();
    state = AuthState.loggedOut;
    notifyListeners();
  }
}
