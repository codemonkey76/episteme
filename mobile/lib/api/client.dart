import 'dart:async';
import 'dart:convert';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

/// Thin client over the episteme REST+SSE API. Auth is the backend's session
/// cookie, captured at login and attached to every request.
enum LoginResult { ok, totpRequired }

class ApiClient {
  ApiClient._();
  static final ApiClient instance = ApiClient._();

  static const _storage = FlutterSecureStorage();

  String _baseUrl = '';
  String? _cookie;

  String get baseUrl => _baseUrl;
  String? get sessionCookie => _cookie;
  bool get hasSession => _cookie != null && _baseUrl.isNotEmpty;

  /// Restore server URL + session cookie from storage. Returns true when a
  /// stored session exists (it may still be expired — callers should verify
  /// via [authStatus]).
  Future<bool> restore() async {
    final prefs = await SharedPreferences.getInstance();
    _baseUrl = prefs.getString('server_url') ?? '';
    try {
      _cookie = await _storage.read(key: 'session_cookie');
    } catch (_) {
      // No keyring (e.g. bare Linux desktop) — session won't persist, but
      // login still works for the lifetime of the process.
      _cookie = null;
    }
    return hasSession;
  }

  Future<void> setServer(String url) async {
    _baseUrl = url.replaceAll(RegExp(r'/+$'), '');
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('server_url', _baseUrl);
  }

  Uri _uri(String path, [Map<String, String>? query]) =>
      Uri.parse('$_baseUrl/api$path').replace(queryParameters: query);

  Map<String, String> _headers({bool json = true}) => {
        if (json) 'Content-Type': 'application/json',
        'Cookie': ?_cookie,
      };

  /// POST /auth/login; captures the session cookie on success. Returns
  /// [LoginResult.totpRequired] when the password was right but the account
  /// has two-factor enabled — call again with the authenticator `code`.
  Future<LoginResult> login(String username, String password,
      {String? code}) async {
    final res = await http.post(
      _uri('/auth/login'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode({
        'username': username,
        'password': password,
        if (code != null && code.isNotEmpty) 'code': code,
      }),
    );
    if (res.statusCode != 200) {
      throw ApiException(_errorMessage(res), res.statusCode);
    }
    final body = jsonDecode(res.body);
    if (body is Map && body['totp_required'] == true) {
      return LoginResult.totpRequired;
    }
    final setCookie = res.headers['set-cookie'];
    if (setCookie == null) {
      throw ApiException('login succeeded but no session cookie returned', 500);
    }
    // Keep only the `name=value` pair of the session cookie.
    _cookie = setCookie.split(';').first;
    try {
      await _storage.write(key: 'session_cookie', value: _cookie);
    } catch (_) {
      // See restore(): missing keyring only costs session persistence.
    }
    return LoginResult.ok;
  }

  Future<void> logout() async {
    try {
      await http.post(_uri('/auth/logout'), headers: _headers());
    } catch (_) {
      // Local logout proceeds even if the server is unreachable.
    }
    _cookie = null;
    try {
      await _storage.delete(key: 'session_cookie');
    } catch (_) {}
  }

  /// GET /auth/status → whether the stored session is still valid.
  Future<bool> authStatus() async {
    final body = await getJson('/auth/status');
    return body['authenticated'] == true;
  }

  Future<Map<String, dynamic>> getJson(String path,
      [Map<String, String>? query]) async {
    final res = await http.get(_uri(path, query), headers: _headers());
    return _decode(res);
  }

  /// GET a non-JSON resource (e.g. a report's raw HTML document).
  Future<String> getText(String path) async {
    final res = await http.get(_uri(path), headers: _headers(json: false));
    if (res.statusCode >= 400) {
      throw ApiException(_errorMessage(res), res.statusCode);
    }
    return utf8.decode(res.bodyBytes);
  }

  Future<Map<String, dynamic>> postJson(String path, Object? body) async {
    final res = await http.post(_uri(path),
        headers: _headers(), body: body == null ? null : jsonEncode(body));
    return _decode(res);
  }

  Future<Map<String, dynamic>> putJson(String path, Object? body) async {
    final res = await http.put(_uri(path),
        headers: _headers(), body: body == null ? null : jsonEncode(body));
    return _decode(res);
  }

  Future<void> patch(String path, [Object? body]) async {
    final res = await http.patch(_uri(path),
        headers: _headers(), body: body == null ? null : jsonEncode(body));
    if (res.statusCode >= 400) {
      throw ApiException(_errorMessage(res), res.statusCode);
    }
  }

  Future<void> delete(String path) async {
    final res = await http.delete(_uri(path), headers: _headers());
    if (res.statusCode >= 400) {
      throw ApiException(_errorMessage(res), res.statusCode);
    }
  }

  /// POST to an SSE endpoint, yielding each decoded `data:` event payload.
  Stream<Map<String, dynamic>> streamSse(String path, Object body) async* {
    final req = http.Request('POST', _uri(path))
      ..headers.addAll(_headers())
      ..body = jsonEncode(body);
    final res = await http.Client().send(req);
    if (res.statusCode >= 400) {
      final text = await res.stream.bytesToString();
      throw ApiException(text, res.statusCode);
    }
    var buffer = '';
    await for (final chunk in res.stream.transform(utf8.decoder)) {
      buffer += chunk;
      while (true) {
        final nl = buffer.indexOf('\n');
        if (nl < 0) break;
        final line = buffer.substring(0, nl).trim();
        buffer = buffer.substring(nl + 1);
        if (!line.startsWith('data:')) continue;
        final payload = line.substring(5).trim();
        if (payload.isEmpty) continue;
        try {
          yield jsonDecode(payload) as Map<String, dynamic>;
        } catch (_) {
          // Skip malformed events; the stream itself stays usable.
        }
      }
    }
  }

  Map<String, dynamic> _decode(http.Response res) {
    if (res.statusCode >= 400) {
      throw ApiException(_errorMessage(res), res.statusCode);
    }
    if (res.body.isEmpty) return const {};
    return jsonDecode(res.body) as Map<String, dynamic>;
  }

  String _errorMessage(http.Response res) {
    try {
      final body = jsonDecode(res.body) as Map<String, dynamic>;
      final err = body['error'];
      if (err is String && err.isNotEmpty) return err;
    } catch (_) {}
    return 'HTTP ${res.statusCode}';
  }
}

class ApiException implements Exception {
  ApiException(this.message, this.status);
  final String message;
  final int status;

  bool get isUnauthorized => status == 401;

  @override
  String toString() => message;
}
