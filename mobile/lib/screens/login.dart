import 'package:flutter/material.dart';
import 'package:provider/provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../main.dart';
import '../state/auth.dart';

class LoginScreen extends StatefulWidget {
  const LoginScreen({super.key});

  @override
  State<LoginScreen> createState() => _LoginScreenState();
}

class _LoginScreenState extends State<LoginScreen> {
  final _server = TextEditingController();
  final _username = TextEditingController();
  final _password = TextEditingController();
  final _code = TextEditingController();
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    SharedPreferences.getInstance().then((prefs) {
      final url = prefs.getString('server_url');
      if (url != null && url.isNotEmpty) _server.text = url;
    });
  }

  Future<void> _submit() async {
    final auth = context.read<AuthStore>();
    setState(() => _busy = true);
    await auth.login(
      _server.text.trim(),
      _username.text.trim(),
      _password.text,
      code: _code.text.trim(),
    );
    if (mounted) setState(() => _busy = false);
  }

  @override
  Widget build(BuildContext context) {
    final auth = context.watch<AuthStore>();
    return Scaffold(
      body: Center(
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(28),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 380),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const Text(
                  'Episteme',
                  textAlign: TextAlign.center,
                  style: TextStyle(
                    fontSize: 26,
                    fontWeight: FontWeight.w600,
                    color: Palette.fg,
                  ),
                ),
                const SizedBox(height: 6),
                const Text(
                  'Your self-hosted AI workspace',
                  textAlign: TextAlign.center,
                  style: TextStyle(fontSize: 13, color: Palette.faint),
                ),
                const SizedBox(height: 28),
                TextField(
                  controller: _server,
                  keyboardType: TextInputType.url,
                  autocorrect: false,
                  decoration: const InputDecoration(
                    labelText: 'Server URL',
                    hintText: 'https://episteme.example.com',
                  ),
                ),
                const SizedBox(height: 12),
                TextField(
                  controller: _username,
                  autocorrect: false,
                  decoration: const InputDecoration(labelText: 'Username'),
                ),
                const SizedBox(height: 12),
                TextField(
                  controller: _password,
                  obscureText: true,
                  onSubmitted: (_) => _submit(),
                  decoration: const InputDecoration(labelText: 'Password'),
                ),
                // 2FA: the password was right — ask for the second factor.
                if (auth.totpRequired) ...[
                  const SizedBox(height: 12),
                  TextField(
                    controller: _code,
                    autofocus: true,
                    keyboardType: TextInputType.number,
                    autofillHints: const [AutofillHints.oneTimeCode],
                    onSubmitted: (_) => _submit(),
                    decoration: const InputDecoration(
                      labelText: 'Two-factor code',
                      hintText: '6-digit or recovery code',
                    ),
                  ),
                ],
                if (auth.error != null) ...[
                  const SizedBox(height: 12),
                  Text(
                    auth.error!,
                    style: const TextStyle(color: Palette.danger, fontSize: 13),
                  ),
                ],
                const SizedBox(height: 20),
                FilledButton(
                  style: FilledButton.styleFrom(
                    backgroundColor: Palette.accentBg,
                    foregroundColor: Palette.accent,
                    padding: const EdgeInsets.symmetric(vertical: 14),
                  ),
                  onPressed: _busy ? null : _submit,
                  child: _busy
                      ? const SizedBox(
                          width: 18,
                          height: 18,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : const Text('Sign in'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
