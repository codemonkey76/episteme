import 'package:html/dom.dart' as dom;
import 'package:html/parser.dart' as html_parser;

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
///  - re-light inline text colours that would vanish against the card,
///    preserving their hue (dark red → light red) so intentional colour
///    survives readably;
///  - give links an explicit palette colour.
/// Colours we can't parse (gradients, images, var()) are left alone.
/// Rewrite an email's inline colour styling in the markup itself so it reads
/// on the current card.
///
/// This exists because `customStylesBuilder` CANNOT reliably override inline
/// styles: flutter_widget_from_html_core applies the element's own `style`
/// attribute AFTER the builder's output (and `!important` is an unimplemented
/// TODO in the package), so an email's `style="color:#000"` wins over our
/// re-lit colour and renders black-on-black on the dark card. Mutating the
/// attributes before the renderer sees them is the only ordering that always
/// works; the rules are the same ones `emailStyleOverrides` applies.
String sanitizeEmailHtml(String html, {required bool dark}) {
  final dom.Document doc;
  try {
    doc = html_parser.parse(html);
  } catch (_) {
    return html; // unparseable: let the renderer do what it can
  }
  final body = doc.body;
  if (body == null) return html;
  for (final e in body.querySelectorAll('*')) {
    _sanitizeElement(e, dark: dark);
  }
  return body.innerHtml;
}

void _sanitizeElement(dom.Element e, {required bool dark}) {
  var style = e.attributes['style'] ?? '';

  // Backgrounds that clash with the card are removed outright.
  final bg = _cssValue(style, 'background-color') ??
      _cssValue(style, 'background') ??
      e.attributes['bgcolor'];
  final bgRgb = parseCssColor(bg);
  if (bgRgb != null) {
    final bgLum = _luminance(bgRgb);
    if (dark ? bgLum > 0.45 : bgLum < 0.45) {
      style = _stripDecl(_stripDecl(style, 'background'), 'background-color');
      e.attributes.remove('bgcolor');
    }
  }

  // Text colours that would vanish are re-lit in place, preserving hue.
  final color = _cssValue(style, 'color') ?? e.attributes['color'];
  final adjusted = readableColor(color, dark: dark);
  if (adjusted != null) {
    style = '${_stripDecl(style, 'color')};color:$adjusted';
    e.attributes.remove('color'); // <font color=…>
  }

  // Links without a usable colour of their own get the palette colour.
  if (e.localName == 'a' &&
      parseCssColor(_cssValue(style, 'color') ?? e.attributes['color']) ==
          null) {
    style = '$style;color:${dark ? '#7ab0ff' : '#1a5fb4'}';
  }

  style = style.replaceAll(RegExp(r'^[;\s]+|[;\s]+$'), '');
  if (style.isEmpty) {
    e.attributes.remove('style');
  } else {
    e.attributes['style'] = style;
  }
}

/// Remove every declaration of `prop` from an inline style string.
/// `background` deliberately doesn't match `background-color:` (no `:` after
/// the bare word there).
String _stripDecl(String style, String prop) => style.replaceAll(
      RegExp('(?:^|;)\\s*$prop\\s*:[^;]*', caseSensitive: false),
      '',
    );

Map<String, String>? emailStyleOverrides(dom.Element e, {required bool dark}) {
  final out = <String, String>{};
  final style = e.attributes['style'] ?? '';

  final bg = _cssValue(style, 'background-color') ??
      _cssValue(style, 'background') ??
      e.attributes['bgcolor'];
  final bgRgb = parseCssColor(bg);
  if (bgRgb != null) {
    final bgLum = _luminance(bgRgb);
    if (dark ? bgLum > 0.45 : bgLum < 0.45) {
      out['background'] = 'none';
      out['background-color'] = 'transparent';
    }
  }

  final color = _cssValue(style, 'color') ?? e.attributes['color'];
  final adjusted = readableColor(color, dark: dark);
  if (adjusted != null) {
    out['color'] = adjusted;
  }

  // Links: the renderer's default can sit poorly on either card; pin a
  // readable palette colour unless the email's own colour already reads fine.
  if (e.localName == 'a' && !out.containsKey('color')) {
    if (parseCssColor(color) == null) {
      out['color'] = dark ? '#7ab0ff' : '#1a5fb4';
    }
  }

  return out.isEmpty ? null : out;
}

/// If `raw` would be hard to read on the current card, return a hue-preserving
/// readable replacement (CSS hex); null = leave the colour alone.
String? readableColor(String? raw, {required bool dark}) {
  final rgb = parseCssColor(raw);
  if (rgb == null) return null;
  final lum = _luminance(rgb);
  // Card luminances: dark #212121 ≈ 0.13, light white = 1.0. Anything within
  // ~0.42 of the card is low-contrast and gets re-lit.
  if (dark && lum < 0.55) {
    return _toHex(_withLightness(rgb, 0.78));
  }
  if (!dark && lum > 0.62) {
    return _toHex(_withLightness(rgb, 0.30));
  }
  return null;
}

/// Last declaration of `prop` in an inline style string, or null.
String? _cssValue(String style, String prop) {
  final re = RegExp('(?:^|;)\\s*$prop\\s*:\\s*([^;]+)', caseSensitive: false);
  final matches = re.allMatches(style).toList();
  return matches.isEmpty ? null : matches.last.group(1)!.trim();
}

/// CSS named colours that show up in real email HTML. Transparent-ish keywords
/// are deliberately absent (parse to null → left alone).
const _named = <String, (int, int, int)>{
  'black': (0, 0, 0),
  'white': (255, 255, 255),
  'red': (255, 0, 0),
  'green': (0, 128, 0),
  'blue': (0, 0, 255),
  'navy': (0, 0, 128),
  'darkblue': (0, 0, 139),
  'royalblue': (65, 105, 225),
  'teal': (0, 128, 128),
  'purple': (128, 0, 128),
  'maroon': (128, 0, 0),
  'darkred': (139, 0, 0),
  'crimson': (220, 20, 60),
  'orange': (255, 165, 0),
  'gold': (255, 215, 0),
  'yellow': (255, 255, 0),
  'olive': (128, 128, 0),
  'darkgreen': (0, 100, 0),
  'forestgreen': (34, 139, 34),
  'gray': (128, 128, 128),
  'grey': (128, 128, 128),
  'darkgray': (169, 169, 169),
  'darkgrey': (169, 169, 169),
  'dimgray': (105, 105, 105),
  'dimgrey': (105, 105, 105),
  'lightgray': (211, 211, 211),
  'lightgrey': (211, 211, 211),
  'silver': (192, 192, 192),
  'gainsboro': (220, 220, 220),
  'whitesmoke': (245, 245, 245),
  'ivory': (255, 255, 240),
  'brown': (165, 42, 42),
};

/// Parse a CSS colour into (r, g, b) 0–255, or null when unparseable or fully
/// transparent. Handles #rgb/#rgba/#rrggbb/#rrggbbaa, rgb()/rgba() with
/// numbers or percentages, hsl()/hsla(), and common named colours.
(int, int, int)? parseCssColor(String? raw) {
  if (raw == null) return null;
  var v = raw.replaceAll('!important', '').trim().toLowerCase();
  if (v.isEmpty) return null;

  final named = _named[v];
  if (named != null) return named;
  switch (v) {
    case 'transparent':
    case 'inherit':
    case 'initial':
    case 'unset':
    case 'currentcolor':
    case 'none':
      return null;
  }

  final hex =
      RegExp(r'^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$').firstMatch(v);
  if (hex != null) {
    var h = hex.group(1)!;
    if (h.length <= 4) h = h.split('').map((c) => '$c$c').join();
    if (h.length == 8) {
      final a = int.parse(h.substring(6, 8), radix: 16);
      if (a == 0) return null; // fully transparent
      h = h.substring(0, 6);
    }
    return (
      int.parse(h.substring(0, 2), radix: 16),
      int.parse(h.substring(2, 4), radix: 16),
      int.parse(h.substring(4, 6), radix: 16),
    );
  }

  final rgb = RegExp(r'^rgba?\(([^)]+)\)$').firstMatch(v);
  if (rgb != null) {
    final parts = rgb.group(1)!.split(RegExp(r'[,/\s]+')).where((p) => p.isNotEmpty).toList();
    if (parts.length < 3) return null;
    double? channel(String p) {
      if (p.endsWith('%')) {
        final pct = double.tryParse(p.substring(0, p.length - 1));
        return pct == null ? null : pct * 2.55;
      }
      return double.tryParse(p);
    }

    final r = channel(parts[0]);
    final g = channel(parts[1]);
    final b = channel(parts[2]);
    if (r == null || g == null || b == null) return null;
    if (parts.length >= 4) {
      final a = parts[3].endsWith('%')
          ? (double.tryParse(parts[3].substring(0, parts[3].length - 1)) ?? 100) / 100
          : double.tryParse(parts[3]) ?? 1;
      if (a == 0) return null;
    }
    return (r.round().clamp(0, 255), g.round().clamp(0, 255), b.round().clamp(0, 255));
  }

  final hsl = RegExp(r'^hsla?\(([^)]+)\)$').firstMatch(v);
  if (hsl != null) {
    final parts = hsl.group(1)!.split(RegExp(r'[,/\s]+')).where((p) => p.isNotEmpty).toList();
    if (parts.length < 3) return null;
    final h = double.tryParse(parts[0].replaceAll('deg', ''));
    final s = double.tryParse(parts[1].replaceAll('%', ''));
    final l = double.tryParse(parts[2].replaceAll('%', ''));
    if (h == null || s == null || l == null) return null;
    if (parts.length >= 4) {
      final a = parts[3].endsWith('%')
          ? (double.tryParse(parts[3].substring(0, parts[3].length - 1)) ?? 100) / 100
          : double.tryParse(parts[3]) ?? 1;
      if (a == 0) return null;
    }
    return _hslToRgb(h % 360, (s / 100).clamp(0.0, 1.0), (l / 100).clamp(0.0, 1.0));
  }

  return null;
}

/// Perceived luminance (0–1).
double _luminance((int, int, int) rgb) =>
    (0.299 * rgb.$1 + 0.587 * rgb.$2 + 0.114 * rgb.$3) / 255;

/// Same hue/saturation, new lightness — the "re-light" operation.
(int, int, int) _withLightness((int, int, int) rgb, double lightness) {
  final (h, s, _) = _rgbToHsl(rgb);
  return _hslToRgb(h, s, lightness);
}

(double, double, double) _rgbToHsl((int, int, int) rgb) {
  final r = rgb.$1 / 255, g = rgb.$2 / 255, b = rgb.$3 / 255;
  final maxC = [r, g, b].reduce((a, c) => a > c ? a : c);
  final minC = [r, g, b].reduce((a, c) => a < c ? a : c);
  final l = (maxC + minC) / 2;
  if (maxC == minC) return (0, 0, l);
  final d = maxC - minC;
  final s = l > 0.5 ? d / (2 - maxC - minC) : d / (maxC + minC);
  double h;
  if (maxC == r) {
    h = (g - b) / d + (g < b ? 6 : 0);
  } else if (maxC == g) {
    h = (b - r) / d + 2;
  } else {
    h = (r - g) / d + 4;
  }
  return (h * 60, s, l);
}

(int, int, int) _hslToRgb(double h, double s, double l) {
  if (s == 0) {
    final v = (l * 255).round().clamp(0, 255);
    return (v, v, v);
  }
  final q = l < 0.5 ? l * (1 + s) : l + s - l * s;
  final p = 2 * l - q;
  double hue(double t) {
    if (t < 0) t += 1;
    if (t > 1) t -= 1;
    if (t < 1 / 6) return p + (q - p) * 6 * t;
    if (t < 1 / 2) return q;
    if (t < 2 / 3) return p + (q - p) * (2 / 3 - t) * 6;
    return p;
  }

  final hh = h / 360;
  return (
    (hue(hh + 1 / 3) * 255).round().clamp(0, 255),
    (hue(hh) * 255).round().clamp(0, 255),
    (hue(hh - 1 / 3) * 255).round().clamp(0, 255),
  );
}

String _toHex((int, int, int) rgb) =>
    '#${rgb.$1.toRadixString(16).padLeft(2, '0')}'
    '${rgb.$2.toRadixString(16).padLeft(2, '0')}'
    '${rgb.$3.toRadixString(16).padLeft(2, '0')}';
