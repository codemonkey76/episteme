import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:flutter_widget_from_html_core/flutter_widget_from_html_core.dart';
import 'package:intl/intl.dart';
import 'package:provider/provider.dart' hide Provider;
import 'package:shared_preferences/shared_preferences.dart';

import '../api/models.dart';
import '../main.dart';
import '../state/email.dart';
import '../state/tasks.dart';
import '../util/email_colors.dart';
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

  // Email presentation: dark card with light text (matches the app, like the
  // web's inverted view) or the original-ish light "paper" card. Inline
  // colours that would vanish against the card are sanitized either way.
  bool _darkEmail = true;

  @override
  void initState() {
    super.initState();
    SharedPreferences.getInstance().then((p) {
      if (mounted) setState(() => _darkEmail = p.getBool('emailDark') ?? true);
    });
    _load();
  }

  void _toggleDarkEmail() {
    setState(() => _darkEmail = !_darkEmail);
    SharedPreferences.getInstance()
        .then((p) => p.setBool('emailDark', _darkEmail));
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

  String _selectedText = '';

  /// Selection context-menu action: the selected text becomes a task, with
  /// the email (sender/subject) attached as notes for context.
  Future<void> _createTaskFromSelection() async {
    final m = widget.summary;
    final text = _selectedText.replaceAll(RegExp(r'\s+'), ' ').trim();
    if (text.isEmpty) return;
    final title = text.length > 140 ? '${text.substring(0, 140)}…' : text;
    var notes =
        'From email — ${m.from.display} <${m.from.address}>: ${m.subject}';
    if (text.length > 140) notes += '\n\n"$text"';
    try {
      await context.read<TasksStore>().create(title: title, notes: notes);
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(SnackBar(
          content: Text('Task added: $title',
              maxLines: 2, overflow: TextOverflow.ellipsis),
          duration: const Duration(seconds: 2),
        ));
      }
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('Task failed: $e')));
      }
    }
  }

  // ── AI summary (streamed, like the web's inline panel) ──────────────────
  bool _showSummary = false;
  bool _summarizing = false;
  String _summaryText = '';
  String? _summaryError;
  StreamSubscription<Map<String, dynamic>>? _summarySub;

  Future<void> _toggleSummary() async {
    setState(() => _showSummary = !_showSummary);
    if (!_showSummary || _summaryText.isNotEmpty || _summarizing) return;
    final store = context.read<EmailStore>();
    if (store.aiProvider.isEmpty) {
      setState(() => _summaryError = 'No AI provider configured.');
      return;
    }
    setState(() {
      _summarizing = true;
      _summaryError = null;
    });
    try {
      _summarySub = store.streamSummary(widget.summary.id).listen((event) {
        if (!mounted) return;
        if (event['type'] == 'token') {
          setState(() => _summaryText += event['text'] as String? ?? '');
        } else if (event['type'] == 'error') {
          setState(() => _summaryError = event['message'] as String?);
        }
      });
      await _summarySub!.asFuture<void>();
    } catch (e) {
      if (mounted) setState(() => _summaryError = '$e');
    } finally {
      if (mounted) setState(() => _summarizing = false);
    }
  }

  // ── Helpdesk ticket from this email ──────────────────────────────────────
  bool _creatingTicket = false;

  Future<void> _createTicket() async {
    if (_creatingTicket) return;
    final store = context.read<EmailStore>();
    final messenger = ScaffoldMessenger.of(context);
    if (store.aiProvider.isEmpty) {
      messenger.showSnackBar(
          const SnackBar(content: Text('No AI provider configured.')));
      return;
    }
    setState(() => _creatingTicket = true);
    messenger.showSnackBar(const SnackBar(
        content: Text('Creating ticket…'),
        duration: Duration(seconds: 60)));
    try {
      final res = await store.createTicket(widget.summary.id);
      messenger.hideCurrentSnackBar();
      messenger.showSnackBar(SnackBar(
          content: Text(
              'Ticket ${res['reference']} created for ${res['client']} (${res['priority']})'),
          duration: const Duration(seconds: 4)));
    } catch (e) {
      messenger.hideCurrentSnackBar();
      messenger
          .showSnackBar(SnackBar(content: Text('Ticket failed: $e')));
    } finally {
      if (mounted) setState(() => _creatingTicket = false);
    }
  }

  @override
  void dispose() {
    _summarySub?.cancel();
    super.dispose();
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
          if (d != null && d.bodyIsHtml)
            IconButton(
              tooltip: _darkEmail
                  ? 'Show original colours'
                  : 'Show email in dark mode',
              icon: Icon(
                _darkEmail ? Icons.light_mode_outlined : Icons.dark_mode_outlined,
                size: 18,
                color: Palette.muted,
              ),
              onPressed: _toggleDarkEmail,
            ),
          PopupMenuButton<String>(
            icon: const Icon(Icons.more_vert, size: 18, color: Palette.muted),
            color: Palette.surface,
            onSelected: (v) {
              if (v == 'summary') _toggleSummary();
              if (v == 'ticket') _createTicket();
            },
            itemBuilder: (_) => [
              PopupMenuItem(
                value: 'summary',
                child: Text(_showSummary ? 'Hide AI summary' : 'AI summary',
                    style: const TextStyle(color: Palette.fg, fontSize: 13.5)),
              ),
              PopupMenuItem(
                value: 'ticket',
                enabled: !_creatingTicket,
                child: Text(
                    _creatingTicket
                        ? 'Creating ticket…'
                        : 'Create helpdesk ticket',
                    style: const TextStyle(color: Palette.fg, fontSize: 13.5)),
              ),
            ],
          ),
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
                    if (_showSummary)
                      Container(
                        margin: const EdgeInsets.only(bottom: 10),
                        padding: const EdgeInsets.all(10),
                        decoration: BoxDecoration(
                          color: const Color(0xFF10161A),
                          border:
                              Border.all(color: const Color(0xFF1E3A4A)),
                          borderRadius: BorderRadius.circular(10),
                        ),
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Row(
                              children: [
                                const Icon(Icons.auto_awesome,
                                    size: 13, color: Color(0xFF6AB8DF)),
                                const SizedBox(width: 6),
                                const Text('AI SUMMARY',
                                    style: TextStyle(
                                        color: Color(0xFF6AB8DF),
                                        fontSize: 11,
                                        letterSpacing: 0.6,
                                        fontWeight: FontWeight.w600)),
                                if (_summarizing) ...[
                                  const SizedBox(width: 8),
                                  const SizedBox(
                                      width: 10,
                                      height: 10,
                                      child: CircularProgressIndicator(
                                          strokeWidth: 2)),
                                ],
                              ],
                            ),
                            const SizedBox(height: 6),
                            if (_summaryError != null)
                              Text(_summaryError!,
                                  style: const TextStyle(
                                      color: Palette.danger, fontSize: 12.5))
                            else if (_summaryText.isNotEmpty)
                              MarkdownBody(
                                data: _summaryText,
                                styleSheet: MarkdownStyleSheet(
                                  p: const TextStyle(
                                      color: Palette.fg,
                                      fontSize: 13,
                                      height: 1.45),
                                  listBullet: const TextStyle(
                                      color: Palette.fg, fontSize: 13),
                                  strong: const TextStyle(
                                      color: Color(0xFFB8D4F0),
                                      fontWeight: FontWeight.w600),
                                ),
                              )
                            else if (!_summarizing)
                              const Text('No summary yet.',
                                  style: TextStyle(
                                      color: Palette.faint, fontSize: 12.5)),
                          ],
                        ),
                      ),
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
                    SelectionArea(
                      onSelectionChanged: (c) =>
                          _selectedText = c?.plainText ?? '',
                      contextMenuBuilder: (ctx, selectableRegionState) {
                        return AdaptiveTextSelectionToolbar.buttonItems(
                          anchors: selectableRegionState.contextMenuAnchors,
                          buttonItems: [
                            ContextMenuButtonItem(
                              label: 'Create task',
                              onPressed: () {
                                selectableRegionState.hideToolbar();
                                _createTaskFromSelection();
                              },
                            ),
                            ...selectableRegionState.contextMenuButtonItems,
                          ],
                        );
                      },
                      // HTML email renders on a card: dark with light text by
                      // default (matching the app), or light "paper" via the
                      // toggle. Either way emailStyleOverrides sanitizes inline
                      // colours that would vanish against the card — the HTML
                      // renderer ignores <style> blocks, so emails styled via
                      // classes otherwise end up with default-coloured text on
                      // their own inline backgrounds (black on black).
                      child: d.bodyIsHtml
                          ? Container(
                              width: double.infinity,
                              padding: const EdgeInsets.all(12),
                              decoration: BoxDecoration(
                                color: _darkEmail
                                    ? const Color(0xFF212121)
                                    : Colors.white,
                                borderRadius: BorderRadius.circular(8),
                              ),
                              child: HtmlWidget(
                                renderedHtml ?? d.body,
                                textStyle: TextStyle(
                                    color: _darkEmail
                                        ? const Color(0xFFDEDEDE)
                                        : const Color(0xFF1A1A1A),
                                    fontSize: 14,
                                    height: 1.45),
                                customStylesBuilder: (e) =>
                                    emailStyleOverrides(e, dark: _darkEmail),
                              ),
                            )
                          : Text(d.body,
                              style: const TextStyle(
                                  color: Palette.fg,
                                  fontSize: 14,
                                  height: 1.5)),
                    ),
                  ],
                ),
    );
  }
}
