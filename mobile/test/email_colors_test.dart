import 'package:episteme_mobile/util/email_colors.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:html/parser.dart' as html_parser;

import 'package:html/dom.dart' as html_dom;

html_dom.Element dom(String markup) =>
    html_parser.parseFragment(markup).children.first;

void main() {
  group('parseCssColor', () {
    test('handles all common formats', () {
      expect(parseCssColor('#fff'), (255, 255, 255));
      expect(parseCssColor('#1a2b3c'), (26, 43, 60));
      expect(parseCssColor('#1a2b3cff'), (26, 43, 60));
      expect(parseCssColor('rgb(10, 20, 30)'), (10, 20, 30));
      expect(parseCssColor('rgb(100%, 0%, 50%)'), (255, 0, 127));
      expect(parseCssColor('rgba(10, 20, 30, 0.5)'), (10, 20, 30));
      expect(parseCssColor('hsl(0, 100%, 50%)'), (255, 0, 0));
      expect(parseCssColor('hsl(120, 100%, 25%)'), (0, 128, 0));
      expect(parseCssColor('navy'), (0, 0, 128));
      expect(parseCssColor('dimgray'), (105, 105, 105));
      expect(parseCssColor('BLACK !important'), (0, 0, 0));
    });

    test('returns null for transparent and unparseable values', () {
      expect(parseCssColor(null), isNull);
      expect(parseCssColor('transparent'), isNull);
      expect(parseCssColor('rgba(0,0,0,0)'), isNull);
      expect(parseCssColor('#0000000'), isNull);
      expect(parseCssColor('var(--brand)'), isNull);
      expect(parseCssColor('linear-gradient(red, blue)'), isNull);
      expect(parseCssColor('currentcolor'), isNull);
    });
  });

  group('readableColor', () {
    test('re-lights dark text in dark mode, preserving hue', () {
      // Pure black has no hue — comes back as a light gray.
      expect(readableColor('#000', dark: true), '#c7c7c7');
      // Mid-gray (#666, the classic invisible-on-dark case) gets lifted.
      expect(readableColor('#666666', dark: true), isNotNull);
      // Dark red stays red, just light enough to read.
      final red = readableColor('darkred', dark: true)!;
      final rgb = parseCssColor(red)!;
      expect(rgb.$1, greaterThan(rgb.$2)); // still red-dominant
      expect(rgb.$1, greaterThan(150)); // and bright
    });

    test('leaves already-readable colours alone', () {
      expect(readableColor('#dedede', dark: true), isNull);
      expect(readableColor('#1a1a1a', dark: false), isNull);
      expect(readableColor('white', dark: true), isNull);
    });

    test('darkens too-light text in light mode', () {
      expect(readableColor('white', dark: false), isNotNull);
      expect(readableColor('ivory', dark: false), isNotNull);
    });
  });

  group('emailStyleOverrides', () {
    test('strips light backgrounds in dark mode', () {
      // (div, not td — the HTML parser drops table cells without a table.)
      final e = dom('<div style="background-color:#ffffff;color:#000">x</div>');
      final out = emailStyleOverrides(e, dark: true)!;
      expect(out['background-color'], 'transparent');
      expect(out['color'], isNotNull); // black text re-lit
    });

    test('strips dark backgrounds in light mode', () {
      final e = dom('<div style="background:#111">x</div>');
      final out = emailStyleOverrides(e, dark: false)!;
      expect(out['background'], 'none');
    });

    test('handles hsl and named colours that used to slip through', () {
      final hsl = dom('<span style="color:hsl(0, 0%, 20%)">x</span>');
      expect(emailStyleOverrides(hsl, dark: true)!['color'], isNotNull);
      final named = dom('<font color="navy">x</font>');
      expect(emailStyleOverrides(named, dark: true)!['color'], isNotNull);
    });

    test('pins a link colour when the email sets none', () {
      final a = dom('<a href="https://x.example">x</a>');
      expect(emailStyleOverrides(a, dark: true)!['color'], '#7ab0ff');
      expect(emailStyleOverrides(a, dark: false)!['color'], '#1a5fb4');
    });

    test('returns null when nothing needs fixing', () {
      final e = dom('<p style="color:#dedede">x</p>');
      expect(emailStyleOverrides(e, dark: true), isNull);
    });
  });
}
