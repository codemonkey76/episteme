import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_widget_from_html_core/flutter_widget_from_html_core.dart';
import 'package:provider/provider.dart';
import 'package:webview_flutter/webview_flutter.dart';

import '../api/models.dart';
import '../main.dart';
import '../state/activity.dart';

/// Full-screen viewer for one research report. The document is fetched over
/// the authed client and rendered from a string, so no cookies ever reach a
/// WebView. Reports are self-contained by construction (inline CSS, data-URI
/// images, no scripts) — JavaScript stays disabled.
class ReportScreen extends StatefulWidget {
  const ReportScreen({super.key, required this.report});
  final Report report;

  @override
  State<ReportScreen> createState() => _ReportScreenState();
}

class _ReportScreenState extends State<ReportScreen> {
  String? _html;
  String? _error;
  WebViewController? _web;

  // WebView is mobile-only; the HtmlWidget fallback keeps reports readable
  // when the app runs on desktop (dev builds) at the cost of SVG charts.
  static bool get _webViewSupported =>
      !kIsWeb &&
      (defaultTargetPlatform == TargetPlatform.android ||
          defaultTargetPlatform == TargetPlatform.iOS);

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final html =
          await context.read<ActivityStore>().reportHtml(widget.report);
      if (!mounted) return;
      if (_webViewSupported) {
        final controller = WebViewController()
          ..setJavaScriptMode(JavaScriptMode.disabled)
          ..setBackgroundColor(Colors.white)
          ..setNavigationDelegate(NavigationDelegate(
            // Source links would navigate the report away; keep it in place.
            onNavigationRequest: (_) => NavigationDecision.prevent,
          ))
          ..loadHtmlString(html);
        setState(() => _web = controller);
      } else {
        setState(() => _html = html);
      }
    } catch (e) {
      if (mounted) setState(() => _error = e.toString());
    }
  }

  bool _sharing = false;

  /// Mint (or reuse) the public link and copy it to the clipboard. Reports are
  /// self-contained, so the recipient needs no account.
  Future<void> _shareLink() async {
    if (_sharing) return;
    setState(() => _sharing = true);
    final messenger = ScaffoldMessenger.of(context);
    final store = context.read<ActivityStore>();
    try {
      final url = await store.shareReport(widget.report);
      await Clipboard.setData(ClipboardData(text: url));
      if (!mounted) return;
      setState(() {}); // reflect the now-shared state in the icon
      messenger.showSnackBar(const SnackBar(
        content: Text('Public link copied — anyone with it can view this report'),
        duration: Duration(seconds: 3),
      ));
    } catch (e) {
      messenger.showSnackBar(SnackBar(content: Text('Share failed: $e')));
    } finally {
      if (mounted) setState(() => _sharing = false);
    }
  }

  Future<void> _revokeShare() async {
    final messenger = ScaffoldMessenger.of(context);
    try {
      await context.read<ActivityStore>().unshareReport(widget.report);
      if (!mounted) return;
      setState(() {});
      messenger.showSnackBar(const SnackBar(
        content: Text('Public link revoked'),
        duration: Duration(seconds: 2),
      ));
    } catch (e) {
      messenger.showSnackBar(SnackBar(content: Text('Revoke failed: $e')));
    }
  }

  Future<void> _delete() async {
    final yes = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        backgroundColor: Palette.surface,
        title: const Text('Delete report?',
            style: TextStyle(color: Palette.fg, fontSize: 16)),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx, false),
            child: const Text('Cancel', style: TextStyle(color: Palette.muted)),
          ),
          TextButton(
            onPressed: () => Navigator.pop(ctx, true),
            child:
                const Text('Delete', style: TextStyle(color: Palette.danger)),
          ),
        ],
      ),
    );
    if (yes != true || !mounted) return;
    try {
      await context.read<ActivityStore>().deleteReport(widget.report);
      if (mounted) Navigator.of(context).pop();
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('Delete failed: $e')));
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text(widget.report.title,
            style: const TextStyle(fontSize: 16),
            overflow: TextOverflow.ellipsis),
        actions: [
          IconButton(
            icon: Icon(
              widget.report.shareToken != null ? Icons.link : Icons.ios_share,
              color: widget.report.shareToken != null ? Palette.accent : Palette.muted,
              size: 20,
            ),
            tooltip: widget.report.shareToken != null
                ? 'Copy public link'
                : 'Create a public link',
            onPressed: _sharing ? null : _shareLink,
          ),
          PopupMenuButton<String>(
            icon: const Icon(Icons.more_vert, color: Palette.muted),
            color: Palette.surface,
            onSelected: (v) {
              if (v == 'revoke') _revokeShare();
              if (v == 'delete') _delete();
            },
            itemBuilder: (_) => [
              if (widget.report.shareToken != null)
                const PopupMenuItem(
                  value: 'revoke',
                  child: Text('Revoke public link',
                      style: TextStyle(color: Palette.fg, fontSize: 13.5)),
                ),
              const PopupMenuItem(
                value: 'delete',
                child: Text('Delete report',
                    style: TextStyle(color: Palette.danger, fontSize: 13.5)),
              ),
            ],
          ),
        ],
      ),
      body: _body(),
    );
  }

  Widget _body() {
    if (_error != null) {
      return Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Text('Could not load report: $_error',
              style: const TextStyle(color: Palette.danger, fontSize: 13)),
        ),
      );
    }
    if (_web != null) return WebViewWidget(controller: _web!);
    if (_html != null) {
      return SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: HtmlWidget(_html!,
            textStyle: const TextStyle(color: Palette.fg, fontSize: 14)),
      );
    }
    return const Center(child: CircularProgressIndicator());
  }
}
