import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/server/server.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/layout.dart';

/// What removing the server leaves.
String get keptKeysWords =>
    'This $deviceName keeps its own keys on its feeders and reaches them directly, '
    'as before the server. Deleting your account on the server revokes those keys.';

/// What a logout revokes with the token: this phone's own keys, and the
/// phones it invited, `invited` holding the server's id of each one's
/// feeder (`null`: the server could not say), named by `feeders`.
String logOutWords(List<String>? invited, List<ListedFeeder> feeders) {
  final String others = switch (invited) {
    null => ', and any phone it invited',
    [] => '',
    [_] => ', and the phone it invited (${_perFeeder(invited, feeders)})',
    _ => ', and the ${invited.length} phones it invited (${_perFeeder(invited, feeders)})',
  };
  return 'The server stops taking this $deviceName and revokes the keys its token holds on your feeders: '
      "this $deviceName's own$others. To keep them, remove the server instead.";
}

/// How many of `invited` are on each feeder: "2 on Kitchen, 1 on Hall".
String _perFeeder(List<String> invited, List<ListedFeeder> feeders) {
  final counts = <String, int>{};
  for (final String id in invited) {
    counts[id] = (counts[id] ?? 0) + 1;
  }
  String name(String id) => feeders.where((feeder) => feeder.id == id).firstOrNull?.name ?? 'a feeder';
  return [for (final MapEntry(:key, :value) in counts.entries) '$value on ${name(key)}'].join(', ');
}

/// The account this phone is logged in to the home server as: who, and the
/// way out.
class AccountScreen extends StatelessWidget {
  const AccountScreen({super.key, required this.server, this.household, this.logInAgain, this.invitedPhones});

  final HomeServer server;

  /// The phones this phone invited, which a logout revokes too: the
  /// server's id of each one's feeder, or `null` when the server did not
  /// say.
  final Future<List<String>?> Function()? invitedPhones;

  /// Opens the home's pets and foods.
  final VoidCallback? household;

  /// Opens the login again, for a phone the server no longer takes.
  final VoidCallback? logInAgain;

  Future<void> _logOut(BuildContext context) async {
    final List<String>? invited = await invitedPhones?.call();
    if (!context.mounted) return;
    final List<ListedFeeder> feeders = switch (server.state) {
      ServerLinked(:final List<ListedFeeder> feeders) => feeders,
      _ => const [],
    };
    if (!await _confirm(context, 'Log out?', logOutWords(invited, feeders))) return;
    final bool revoked = await server.logOut();
    if (!context.mounted) return;
    if (!revoked) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text("The server couldn't be reached, so this $deviceName's token works until it is revoked there."),
        ),
      );
    }
    Navigator.pop(context);
  }

  Future<void> _remove(BuildContext context) async {
    final words =
        "The server keeps this $deviceName's token until it is revoked there; log out instead to end it now. "
        '$keptKeysWords';
    if (!await _confirm(context, 'Remove the server?', words)) return;
    await server.removeServer();
    if (context.mounted) Navigator.pop(context);
  }

  Future<bool> _confirm(BuildContext context, String title, String text) async =>
      await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: Text(title),
          content: Text(text),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
            TextButton(onPressed: () => Navigator.pop(context, true), child: Text(title.replaceAll('?', ''))),
          ],
        ),
      ) ??
      false;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: server,
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final ServerState state = server.state;
      if (state is! ServerLinked) {
        return const ServerPage(title: 'Home server', children: [Muted('No home server is set up.')]);
      }
      final ServerAccount? account = state.account;
      return ServerPage(
        title: 'Home server',
        children: [
          Panel(
            padding: const EdgeInsets.all(Space.s4),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  state.link.user,
                  style: TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: c.text),
                ),
                const SizedBox(height: Space.s1),
                Muted(state.link.url),
                if (account != null) Muted(roleWords(account.role)),
              ],
            ),
          ),
          const SizedBox(height: Space.s3),
          if (state.problem case final ServerProblem problem) ...[
            Notice(
              tone: problem == ServerProblem.signedOut ? Tone.fault : Tone.warning,
              text: switch (problem) {
                ServerProblem.unreachable =>
                  'The server is not answering. Feeders this $deviceName reaches go directly.',
                ServerProblem.signedOut =>
                  'The server no longer takes this $deviceName, so nothing is sent on its own keys either. '
                      'Log in again, or remove the server.',
                ServerProblem.other => 'The server answered something this app does not read.',
                ServerProblem.plainHttp =>
                  "This server's address is plain http:// beyond the home network, so this $deviceName's "
                      'token has already crossed the network unencrypted, and it never expires. '
                      'Remove the server, add it again by its https:// address, then revoke the old '
                      "token for this $deviceName from the server's token list.",
                ServerProblem.storage =>
                  "This $deviceName's storage couldn't read the server's sign-in. Nothing was deleted. "
                      'Close the app and open it again; if this stays, restart the $deviceName.',
              },
            ),
            const SizedBox(height: Space.s3),
            if (problem == ServerProblem.signedOut && logInAgain != null) ...[
              FilledButton(onPressed: logInAgain, child: const Text('Log in again')),
              const SizedBox(height: Space.s3),
            ],
          ],
          if (account?.role == ServerRole.viewer) ...[
            const Notice(tone: Tone.neutral, text: 'A viewer sees the feeders and their history, and feeds none.'),
            const SizedBox(height: Space.s3),
          ],
          if (account?.role == ServerRole.unknown) ...[
            const Notice(
              tone: Tone.neutral,
              text: 'This account has a role this app does not know, so here it only looks. Update this app.',
            ),
            const SizedBox(height: Space.s3),
          ],
          if (household case final VoidCallback open)
            OutlinedButton(onPressed: open, child: const Text('Pets and foods')),
          const SizedBox(height: Space.s5),
          Muted(keptKeysWords),
          const SizedBox(height: Space.s3),
          FilledButton(onPressed: () => unawaited(_logOut(context)), child: const Text('Log out')),
          const SizedBox(height: Space.s2),
          OutlinedButton(onPressed: () => unawaited(_remove(context)), child: const Text('Remove server')),
        ],
      );
    },
  );
}
