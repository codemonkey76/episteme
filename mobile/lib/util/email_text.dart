/// Plain-text extraction from email bodies, ported from the web frontend
/// (Email.vue): HTML → text preserving line breaks, then truncated at the
/// first quoted-reply marker. Used to build AI-draft and reply_context
/// payloads from just the latest message in a thread.
library;

import '../api/models.dart';

String latestMessageText(MessageDetail m) {
  final text = m.bodyIsHtml ? htmlToText(m.body) : m.body;
  return truncateAtQuote(text);
}

String htmlToText(String html) {
  var s = html
      .replaceAll(
          RegExp(r'<(script|style)[\s\S]*?</\1>', caseSensitive: false), ' ')
      .replaceAll(
          RegExp(r'<blockquote[\s\S]*?</blockquote>', caseSensitive: false),
          '\n__QUOTE__\n')
      .replaceAll(
          RegExp(r'</(p|div|tr|li|h[1-6])>', caseSensitive: false), '\n')
      .replaceAll(RegExp(r'<br\s*/?>', caseSensitive: false), '\n')
      .replaceAll(RegExp(r'<[^>]+>'), '');
  s = decodeEntities(s);
  return s
      .split('\n')
      .map((l) => l.trim())
      .join('\n')
      .replaceAll(RegExp(r'\n{3,}'), '\n\n')
      .trim();
}

/// Decode the common HTML entities (no DOM available in Dart).
String decodeEntities(String s) {
  s = s.replaceAllMapped(RegExp(r'&#(\d+);'), (m) {
    final code = int.tryParse(m[1]!);
    return code != null ? String.fromCharCode(code) : m[0]!;
  });
  s = s.replaceAllMapped(RegExp(r'&#x([0-9a-fA-F]+);'), (m) {
    final code = int.tryParse(m[1]!, radix: 16);
    return code != null ? String.fromCharCode(code) : m[0]!;
  });
  const named = {
    '&nbsp;': ' ',
    '&lt;': '<',
    '&gt;': '>',
    '&quot;': '"',
    '&#39;': "'",
    '&apos;': "'",
    '&rsquo;': '’',
    '&lsquo;': '‘',
    '&rdquo;': '”',
    '&ldquo;': '“',
    '&ndash;': '–',
    '&mdash;': '—',
    '&hellip;': '…',
    '&amp;': '&', // last, so double-encoded entities survive one pass
  };
  named.forEach((k, v) => s = s.replaceAll(k, v));
  return s;
}

String truncateAtQuote(String text) {
  final markers = [
    RegExp(r'^From:\s.+\r?\n\s*(Sent|Date):\s',
        multiLine: true, caseSensitive: false),
    RegExp(r'^-{2,}\s*Original Message\s*-{2,}',
        multiLine: true, caseSensitive: false),
    RegExp(r'^On\s[\s\S]{1,200}?\bwrote:\s*$',
        multiLine: true, caseSensitive: false),
    RegExp(r'^_{5,}\s*$', multiLine: true),
    RegExp(r'^>{1,}\s', multiLine: true),
    RegExp(r'\n__QUOTE__'),
  ];
  var cut = text.length;
  for (final re in markers) {
    final match = re.firstMatch(text);
    if (match != null && match.start > 0 && match.start < cut) {
      cut = match.start;
    }
  }
  final head = text.substring(0, cut).replaceAll('__QUOTE__', '').trim();
  return head.length >= 2 ? head : text.replaceAll('__QUOTE__', '').trim();
}
