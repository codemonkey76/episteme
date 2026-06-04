import 'package:flutter_test/flutter_test.dart';
import 'package:episteme_mobile/util/email_text.dart';

void main() {
  test('htmlToText preserves line breaks and decodes entities', () {
    final t = htmlToText('<div>Hi Len,</div><div>It&#39;s done &amp; dusted</div><br><p>Cheers</p>');
    expect(t, 'Hi Len,\nIt\'s done & dusted\n\nCheers');
  });

  test('truncateAtQuote cuts at Outlook From/Sent header', () {
    final t = truncateAtQuote(
        'I\'ll get this done tonight.\nFrom: Len Groves <len@x.com>\nSent: Thursday\nold stuff');
    expect(t, 'I\'ll get this done tonight.');
  });

  test('truncateAtQuote cuts at blockquote marker', () {
    final t = truncateAtQuote('New text\n__QUOTE__\nquoted history');
    expect(t, 'New text');
  });

  test('truncateAtQuote keeps full text when cut would empty it', () {
    final t = truncateAtQuote('> all quoted\n> lines');
    expect(t.isNotEmpty, true);
  });
}
