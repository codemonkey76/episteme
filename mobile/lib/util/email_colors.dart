import 'package:html/dom.dart' as dom;

/// Per-element style overrides that keep HTML email readable on the app's
/// card, in either presentation mode.
///
/// flutter_widget_from_html_core ignores `<style>`-block CSS, so emails that
/// set text colours via classes lose them (falling back to our default text
/// style) while inline backgrounds/bgcolor attributes still apply — the
/// classic result is near-black default text inside an email's own black
/// container. Strategy:
///  - drop inline backgrounds that clash with the card (dark ones in light
///    mode, light ones in dark mode);
///  - flip inline text colours that would vanish against the card.
/// Colours we can't parse (gradients, images, var()) are left alone.
Map<String, String>? emailStyleOverrides(dom.Element e, {required bool dark}) {
  final out = <String, String>{};
  final style = e.attributes['style'] ?? '';

  final bg = _cssValue(style, 'background-color') ??
      _cssValue(style, 'background') ??
      e.attributes['bgcolor'];
  final bgLum = _luminance(bg);
  if (bgLum != null && (dark ? bgLum > 0.45 : bgLum < 0.45)) {
    out['background'] = 'none';
    out['background-color'] = 'transparent';
  }

  final color = _cssValue(style, 'color') ?? e.attributes['color'];
  final cLum = _luminance(color);
  if (cLum != null) {
    if (dark && cLum < 0.35) out['color'] = '#dedede';
    if (!dark && cLum > 0.75) out['color'] = '#1a1a1a';
  }

  return out.isEmpty ? null : out;
}

/// Last declaration of `prop` in an inline style string, or null.
String? _cssValue(String style, String prop) {
  final re = RegExp('(?:^|;)\\s*$prop\\s*:\\s*([^;]+)', caseSensitive: false);
  final matches = re.allMatches(style).toList();
  return matches.isEmpty ? null : matches.last.group(1)!.trim();
}

/// Perceived luminance (0–1) of a CSS colour, or null when unparseable /
/// fully transparent.
double? _luminance(String? raw) {
  if (raw == null) return null;
  var v = raw.replaceAll('!important', '').trim().toLowerCase();
  switch (v) {
    case 'white':
      return 1;
    case 'black':
      return 0;
    case 'transparent':
    case 'inherit':
    case 'initial':
    case 'unset':
    case 'none':
      return null;
  }
  final hex = RegExp(r'^#([0-9a-f]{3}|[0-9a-f]{6})$').firstMatch(v);
  if (hex != null) {
    var h = hex.group(1)!;
    if (h.length == 3) h = h.split('').map((c) => '$c$c').join();
    final r = int.parse(h.substring(0, 2), radix: 16);
    final g = int.parse(h.substring(2, 4), radix: 16);
    final b = int.parse(h.substring(4, 6), radix: 16);
    return (0.299 * r + 0.587 * g + 0.114 * b) / 255;
  }
  final rgb = RegExp(r'^rgba?\(([^)]+)\)$').firstMatch(v);
  if (rgb != null) {
    final parts = rgb
        .group(1)!
        .split(',')
        .map((p) => double.tryParse(p.trim()))
        .toList();
    if (parts.length >= 3 &&
        parts[0] != null &&
        parts[1] != null &&
        parts[2] != null) {
      if (parts.length == 4 && (parts[3] ?? 1) == 0) return null;
      return (0.299 * parts[0]! + 0.587 * parts[1]! + 0.114 * parts[2]!) / 255;
    }
  }
  return null;
}
