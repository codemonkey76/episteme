import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import 'push.dart';
import 'screens/login.dart';
import 'screens/shell.dart';
import 'state/activity.dart';
import 'state/auth.dart';
import 'state/chat.dart';
import 'state/email.dart';
import 'state/notes.dart';
import 'state/shipments.dart';
import 'state/tasks.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await Push.init();
  runApp(const EpistemeApp());
}

/// Episteme's dark palette, matched to the web frontend.
class Palette {
  static const bg = Color(0xFF0D0D0D);
  static const surface = Color(0xFF181818);
  static const raised = Color(0xFF262626);
  static const fg = Color(0xFFD0D0D0);
  static const muted = Color(0xFF808080);
  static const faint = Color(0xFF585858);
  static const accent = Color(0xFF7AB0FF);
  static const accentBg = Color(0xFF1E3A6E);
  static const danger = Color(0xFFDF7A7A);
  static const ok = Color(0xFF6ECF8E);
  static const warn = Color(0xFFE0B060);
}

class EpistemeApp extends StatelessWidget {
  const EpistemeApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MultiProvider(
      providers: [
        ChangeNotifierProvider(create: (_) => AuthStore()..init()),
        ChangeNotifierProvider(create: (_) => TasksStore()),
        ChangeNotifierProvider(create: (_) => ChatStore()),
        ChangeNotifierProvider(create: (_) => EmailStore()),
        ChangeNotifierProvider(create: (_) => NotesStore()),
        ChangeNotifierProvider(create: (_) => ShipmentsStore()),
        ChangeNotifierProvider(create: (_) => ActivityStore()),
      ],
      child: MaterialApp(
        title: 'Episteme',
        debugShowCheckedModeBanner: false,
        theme: _theme(),
        home: const _Root(),
      ),
    );
  }

  ThemeData _theme() {
    final base = ThemeData.dark(useMaterial3: true);
    return base.copyWith(
      scaffoldBackgroundColor: Palette.bg,
      colorScheme: base.colorScheme.copyWith(
        primary: Palette.accent,
        secondary: Palette.accent,
        surface: Palette.surface,
        error: Palette.danger,
      ),
      appBarTheme: const AppBarTheme(
        backgroundColor: Palette.bg,
        foregroundColor: Palette.fg,
        elevation: 0,
        centerTitle: false,
      ),
      navigationBarTheme: NavigationBarThemeData(
        backgroundColor: Palette.surface,
        indicatorColor: Palette.accentBg,
        iconTheme: WidgetStateProperty.resolveWith(
          (states) => IconThemeData(
            color: states.contains(WidgetState.selected)
                ? Palette.accent
                : Palette.muted,
          ),
        ),
        labelTextStyle: WidgetStateProperty.resolveWith(
          (states) => TextStyle(
            fontSize: 12,
            color: states.contains(WidgetState.selected)
                ? Palette.accent
                : Palette.muted,
          ),
        ),
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: Palette.surface,
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(8),
          borderSide: const BorderSide(color: Palette.raised),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(8),
          borderSide: const BorderSide(color: Palette.raised),
        ),
      ),
      floatingActionButtonTheme: const FloatingActionButtonThemeData(
        backgroundColor: Palette.accentBg,
        foregroundColor: Palette.accent,
      ),
    );
  }
}

class _Root extends StatelessWidget {
  const _Root();

  @override
  Widget build(BuildContext context) {
    final auth = context.watch<AuthStore>();
    switch (auth.state) {
      case AuthState.unknown:
        return const Scaffold(body: Center(child: CircularProgressIndicator()));
      case AuthState.loggedOut:
        return const LoginScreen();
      case AuthState.loggedIn:
        return const ShellScreen();
    }
  }
}
