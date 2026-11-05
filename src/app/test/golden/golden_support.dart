import 'dart:io';

import 'package:flutter/material.dart' hide Card;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/shell.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'comparator.dart';

/// The comparator and fonts every golden file sets up first.
void setUpGoldens(String file) => setUpAll(() async {
  final previous = goldenFileComparator as LocalFileComparator;
  goldenFileComparator = LinuxGoldenComparator(previous.basedir.resolve(file));
  await loadGoldenFonts();
});

/// `screen` pushed over a blank route, so its back or close button shows
/// as it does on a phone, where something is always under it.
Widget overBlank(Widget screen) => Navigator(
  onGenerateInitialRoutes: (_, _) => [
    MaterialPageRoute<void>(builder: (_) => const Scaffold()),
    MaterialPageRoute<void>(builder: (_) => screen),
  ],
);

/// Pixels differ between font rasterizers by as much as a real change, so
/// Linux owns these files; macOS runs them in a Linux container
/// (`make check-app`). x86_64 and arm64 Linux agree within the comparator's
/// tolerance.
final bool skipGoldens = !Platform.isLinux;

/// The app's own fonts: the test default draws every letter as a box.
Future<void> loadGoldenFonts() async {
  final Map<String, List<String>> families = {
    Fonts.family: ['Inter-Regular', 'Inter-Medium', 'Inter-SemiBold', 'Inter-ExtraBold'],
    Fonts.mono: ['JetBrainsMono-Regular'],
  };
  for (final MapEntry(key: family, value: files) in families.entries) {
    final loader = FontLoader(family);
    for (final file in files) {
      loader.addFont(rootBundle.load('assets/fonts/$file.ttf'));
    }
    await loader.load();
  }
  await (FontLoader('MaterialIcons')..addFont(rootBundle.load('fonts/MaterialIcons-Regular.otf'))).load();
}

const phoneSize = Size(412, 915);
const smallPhone = Size(360, 740);

/// Large text, as a person who sets Android's font size up a step or two sees.
const largeText = 1.3;

Future<void> showPhone(
  WidgetTester tester,
  Widget screen,
  Brightness brightness, {
  Size size = phoneSize,
  double text = 1,
  double ratio = 1,
}) async {
  frame(tester, size: size, text: text, ratio: ratio);
  await tester.pumpWidget(themed(screen, brightness: brightness));
  await settle(tester);
}

/// `size` in logical pixels, drawn `ratio` physical pixels to each.
void frame(WidgetTester tester, {Size size = phoneSize, double text = 1, double ratio = 1}) {
  tester.view.physicalSize = size * ratio;
  tester.view.devicePixelRatio = ratio;
  tester.platformDispatcher.textScaleFactorTestValue = text;
  addTearDown(tester.view.reset);
  addTearDown(tester.platformDispatcher.clearTextScaleFactorTestValue);
}

/// The fixture's feeder, its session answering as [answerAsFull] does.
Future<FeederController> goldenFeeder(WidgetTester tester, {Card? card}) async {
  final (FeederController feeder, FakeSession session) = await oneFeeder(tester, phone: fixturePhone());
  answerAsFull(session);
  session.status(card ?? fullCard());
  await tester.pump();
  return feeder;
}

/// Decode the screen's photos for real: the fake clock never lets a decode
/// finish, and a photo no earlier test cached would draw as nothing.
Future<void> decodeImages(WidgetTester tester) async {
  for (final Element element in find.byType(Image).evaluate()) {
    final ImageProvider provider = (element.widget as Image).image;
    await tester.runAsync(() => precacheImage(provider, element));
  }
  await settle(tester);
}

/// The feeder's own screen at `tab`, with its name and tabs.
FeederShell shell(FeederController feeder, int tab) => FeederShell(feeder: feeder, initialTab: tab);

/// A golden on Linux, with the app's clock stopped at the fixture's moment.
void golden(String name, Future<void> Function(WidgetTester tester) body) =>
    testWidgets(name, skip: skipGoldens, (tester) => atFixtureNow(() => body(tester)));

Future<void> matchesGolden(String name, Brightness brightness) =>
    expectLater(find.byType(MaterialApp), matchesGoldenFile('files/$name.${brightness.name}.png'));
