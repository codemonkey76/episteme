import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_widget_from_html_core/flutter_widget_from_html_core.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart' hide Provider;

import '../api/models.dart';
import '../main.dart';
import '../state/email.dart';
import '../util/email_text.dart';
import 'compose_screen.dart';

class MessageScreen extends StatefulWidget {
  const MessageScreen({super.key, required this.summary});
  final MessageSummary summary;

  @override
  State<MessageScreen> createState() => _MessageScreenState();
}

class _MessageScreenState extends State<MessageScreen> {
  MessageDetail? detail;
  String? renderedHtml;
  String? error;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final store = context.read<EmailStore>();
    try {
      final d = await store.getMessage(widget.summary);
      if (!mounted) return;
      setState(() => detail = d);
      if (d.bodyIsHtml && d.body.contains('cid:')) {
        await _inlineCidImages(store, d);
      }
    } catch (e) {
      if (mounted) setState(() => error = e.toString());
    }
  }

  /// Replace cid: refs with data: URIs fetched through the authed API —
  /// the HTML renderer can't send the session cookie itself. Same two-pass
  /// matching as the web (exact name/contentId, then positional inline).
  Future<void> _inlineCidImages(EmailStore store, MessageDetail d) async {
    try {
      final atts = await store.listAttachments(d.summary.id);
      final cids = RegExp(r'''cid:([^"'\s)>]+)''')
          .allMatches(d.body)
          .map((m) => m.group(1)!)
          .toSet()
          .toList();
      final used = <String>{};
      final assigned = <String, Attachment>{};
      for (final cid in cids) {
        final prefix = cid.split('@').first;
        for (final a in atts) {
          final cidMatch = a.contentId?.replaceAll(RegExp(r'^<|>$'), '') == cid;
          if (cidMatch || a.name == cid || a.name == prefix) {
            used.add(a.id);
            assigned[cid] = a;
            break;
          }
        }
      }
      for (final cid in cids) {
        if (assigned.containsKey(cid)) continue;
        for (final a in atts) {
          if (a.isInline && !used.contains(a.id)) {
            used.add(a.id);
            assigned[cid] = a;
            break;
          }
        }
      }
      var html = d.body;
      for (final entry in assigned.entries) {
        final bytes = await store.fetchAttachment(d.summary.id, entry.value);
        if (bytes == null) continue;
        final dataUri =
            'data:${bytes.$2};base64,${base64Encode(bytes.$1)}';
        html = html.replaceAll('cid:${entry.key}', dataUri);
      }
      if (mounted) setState(() => renderedHtml = html);
    } catch (_) {
      // Images are cosmetic; the text body already rendered.
    }
  }

  void _compose(String mode) {
    final d = detail;
    if (d == null) return;
    showComposeScreen(
      context,
      detail: d,
      mode: mode,
      latestText: latestMessageText(d),
    );
  }

  bool _markingDone = false;
  Future<void> _markDone() async {
    if (_markingDone) return;
    setState(() => _markingDone = true);
    final store = context.read<EmailStore>();
    try {
      await store.markDone(widget.summary.id);
      if (mounted) Navigator.pop(context);
    } catch (e) {
      if (mounted) {
        setState(() {
          _markingDone = false;
          error = 'Mark done failed: $e';
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final d = detail;
    final s = widget.summary;
    return Scaffold(
      backgroundColor: Palette.bg,
      appBar: AppBar(
        title: Text(s.subject,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: const TextStyle(fontSize: 16)),
        actions: [
          TextButton.icon(
            onPressed: _markingDone ? null : _markDone,
            icon: _markingDone
                ? const SizedBox(
                    width: 13,
                    height: 13,
                    child: CircularProgressIndicator(strokeWidth: 2))
                : const Icon(Icons.done_all, size: 16, color: Palette.ok),
            label: const Text('Done',
                style: TextStyle(color: Palette.ok, fontSize: 13)),
          ),
        ],
      ),
      bottomNavigationBar: d == null
          ? null
          : SafeArea(
              child: Padding(
                padding: const EdgeInsets.fromLTRB(12, 6, 12, 10),
                child: Row(
                  children: [
                    Expanded(
                      flex: 5,
                      child: FilledButton.icon(
                        style: FilledButton.styleFrom(
                          backgroundColor: const Color(0xFF23304A),
                          foregroundColor: const Color(0xFFA0C8FF),
                          padding: const EdgeInsets.symmetric(horizontal: 6),
                        ),
                        icon: const Icon(Icons.auto_awesome, size: 15),
                        label: const Text('AI reply',
                            style: TextStyle(fontSize: 12.5)),
                        onPressed: () => _compose('ai'),
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      flex: 4,
                      child: FilledButton.icon(
                        style: FilledButton.styleFrom(
                          backgroundColor: Palette.surface,
                          foregroundColor: Palette.muted,
                          padding: const EdgeInsets.symmetric(horizontal: 6),
                        ),
                        icon: const Icon(Icons.reply, size: 15),
                        label: const Text('Reply',
                            style: TextStyle(fontSize: 12.5)),
                        onPressed: () => _compose('reply'),
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      flex: 3,
                      child: FilledButton.icon(
                        style: FilledButton.styleFrom(
                          backgroundColor: Palette.surface,
                          foregroundColor: Palette.muted,
                          padding: const EdgeInsets.symmetric(horizontal: 6),
                        ),
                        icon: const Icon(Icons.reply_all, size: 15),
                        label:
                            const Text('All', style: TextStyle(fontSize: 12.5)),
                        onPressed: () => _compose('replyAll'),
                      ),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      flex: 3,
                      child: FilledButton.icon(
                        style: FilledButton.styleFrom(
                          backgroundColor: Palette.surface,
                          foregroundColor: Palette.muted,
                          padding: const EdgeInsets.symmetric(horizontal: 6),
                        ),
                        icon: const Icon(Icons.forward, size: 15),
                        label:
                            const Text('Fwd', style: TextStyle(fontSize: 12.5)),
                        onPressed: () => _compose('forward'),
                      ),
                    ),
                  ],
                ),
              ),
            ),
      body: error != null
          ? Center(
              child: Text(error!,
                  style: const TextStyle(color: Palette.danger, fontSize: 13)))
          : d == null
              ? const Center(child: CircularProgressIndicator())
              : ListView(
                  padding: const EdgeInsets.fromLTRB(16, 8, 16, 16),
                  children: [
                    Text(s.from.display,
                        style: const TextStyle(
                            color: Palette.fg,
                            fontSize: 14.5,
                            fontWeight: FontWeight.w600)),
                    Text(s.from.address,
                        style: const TextStyle(
                            color: Palette.faint, fontSize: 12)),
                    const SizedBox(height: 2),
                    Text(
                      'to ${d.to.map((a) => a.display).join(', ')}'
                      '${d.cc.isNotEmpty ? ' · cc ${d.cc.map((a) => a.display).join(', ')}' : ''}',
                      style:
                          const TextStyle(color: Palette.faint, fontSize: 12),
                    ),
                    if (s.received != null)
                      Text(
                          DateFormat('EEE d MMM yyyy, h:mm a')
                              .format(s.received!),
                          style: const TextStyle(
                              color: Palette.faint, fontSize: 12)),
                    const Divider(color: Color(0xFF1E1E1E), height: 24),
                    if (d.bodyIsHtml)
                      HtmlWidget(
                        renderedHtml ?? d.body,
                        textStyle: const TextStyle(
                            color: Palette.fg, fontSize: 14, height: 1.45),
                      )
                    else
                      SelectableText(d.body,
                          style: const TextStyle(
                              color: Palette.fg, fontSize: 14, height: 1.5)),
                  ],
                ),
    );
  }
}
