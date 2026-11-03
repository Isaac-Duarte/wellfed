import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/app.dart';
import 'package:wellfed/screens/home/welcome.dart';
import 'package:wellfed/screens/settings/app_settings.dart';
import 'package:wellfed/state/settings/backups.dart';
import 'package:wellfed/theme/theme.dart';

import '../support/fakes.dart';
import 'feeder_support.dart';

void main() {
  group('Settings gear', () {
    Future<GoRouter> app(WidgetTester tester, Widget root) async {
      feederPhone(tester);
      final book = FakeBook();
      final GoRouter router = wellfedRouter(
        book: book,
        root: (context) => root,
        pages: {
          '/settings': (context) =>
              AppSettingsScreen(book: book, newBackups: () => BackupsFlow(book, FakeBackupPorts())),
        },
      );
      addTearDown(router.dispose);
      await tester.pumpWidget(MaterialApp.router(theme: wellfedTheme(Brightness.light), routerConfig: router));
      await settle(tester);
      return router;
    }

    testWidgets('Welcome opens the settings before there is a feeder', (tester) async {
      await app(tester, const WelcomeScreen());

      await feederTap(tester, find.byTooltip('Settings'));
      expect(find.text('APPEARANCE'), findsOneWidget);

      await feederTap(tester, find.byTooltip('Back'));
      expect(find.text('APPEARANCE'), findsNothing);
      expect(find.byType(WelcomeScreen), findsOneWidget);
    });

    testWidgets('settings reached with nothing under them go back home', (tester) async {
      final GoRouter router = await app(tester, const WelcomeScreen());
      router.go('/settings');
      await settle(tester);
      expect(find.byType(WelcomeScreen), findsNothing);

      await feederTap(tester, find.byTooltip('Back'));
      expect(find.byType(WelcomeScreen), findsOneWidget);
    });
  });
}
