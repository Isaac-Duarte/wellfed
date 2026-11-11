import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/feeder/history.dart';
import 'package:wellfed/screens/server/account.dart';
import 'package:wellfed/screens/server/add_server.dart';
import 'package:wellfed/screens/server/feeder_food.dart';
import 'package:wellfed/screens/server/household.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/fleet/adopt.dart';
import 'package:wellfed/state/server/add_server.dart';
import 'package:wellfed/state/server/feeder_food.dart';
import 'package:wellfed/state/server/household.dart';
import 'package:wellfed/state/server/pet.dart';
import 'package:wellfed/state/server/server.dart';
import 'package:wellfed/state/server/server_history.dart';
import 'package:wellfed/widgets/desktop.dart';

/// The owner's server's screens, and the controllers they make. The app
/// builds one; a screen that has none shows the app as it is with no server.
/// A maker gives `null` while no session is open, and the screen says so.
class ServerScreens {
  const ServerScreens({
    required this.server,
    required this.adopt,
    required this.newAddServer,
    required this.newHousehold,
    required this.newPet,
    required this.newFood,
    required this.newHistory,
    this.invitedPhones,
  });

  final HomeServer server;

  /// Putting feeders this phone paired onto the server.
  final AdoptFlow adopt;

  final AddServerFlow Function() newAddServer;
  final HouseholdFlow? Function() newHousehold;
  final PetFlow? Function(ServerPet pet) newPet;

  /// One feeder's food, by the server's id for it.
  final FeederFoodFlow? Function(String feeder) newFood;

  /// One feeder's long history, by the server's id for it.
  final ServerHistoryFlow? Function(String feeder) newHistory;

  /// The phones this phone invited, which a logout revokes too.
  final Future<List<String>?> Function()? invitedPhones;

  /// The account screen, or adding a server when there is none.
  Future<void> open(BuildContext context) async {
    if (server.state is! ServerLinked) {
      await _push(context, (_) => FormWidth(child: AddServerScreen(newFlow: newAddServer)));
      if (server.state is! ServerLinked || !context.mounted) return;
    }
    await _push(
      context,
      (context) => FormWidth(
        child: AccountScreen(
          server: server,
          household: () => unawaited(openHousehold(context)),
          logInAgain: () => unawaited(_push(context, (_) => FormWidth(child: AddServerScreen(newFlow: newAddServer)))),
          invitedPhones: invitedPhones,
        ),
      ),
    );
  }

  /// The home's pets and foods.
  Future<void> openHousehold(BuildContext context) => _push(
    context,
    (_) => switch (newHousehold()) {
      final HouseholdFlow flow => HouseholdScreen(
        flow: flow,
        newPet: newPet,
        feeders: switch (server.state) {
          ServerLinked(:final List<ListedFeeder> feeders) => feeders,
          _ => const [],
        },
      ),
      null => const _NoSession(title: 'Pets and foods'),
    },
  );

  /// What `feeder` holds, and its refills.
  Future<void> openFood(BuildContext context, FeederController feeder) => _push(
    context,
    (_) => switch (feeder.state.serverId) {
      final String id? => switch (newFood(id)) {
        final FeederFoodFlow flow => FeederFoodScreen(flow: flow, name: feeder.state.known.name),
        null => const _NoSession(title: 'Food'),
      },
      null => const _NoSession(title: 'Food'),
    },
  );

  /// What the server kept of `feeder`'s history.
  Future<void> openHistory(BuildContext context, FeederController feeder) => _push(
    context,
    (_) => switch (feeder.state.serverId) {
      final String id? => switch (newHistory(id)) {
        final ServerHistoryFlow flow => ServerHistoryScreen(flow: flow, feeder: feeder),
        null => const _NoSession(title: 'Full history'),
      },
      null => const _NoSession(title: 'Full history'),
    },
  );

  static Future<void> _push(BuildContext context, WidgetBuilder page) =>
      Navigator.of(context).push(MaterialPageRoute<void>(builder: page));
}

class _NoSession extends StatelessWidget {
  const _NoSession({required this.title});

  final String title;

  @override
  Widget build(BuildContext context) =>
      ServerPage(title: title, children: const [Muted('The home server is not connected right now.')]);
}

/// The history the owner's server kept of one feeder, back further than the
/// feeder's own ring.
class ServerHistoryScreen extends StatefulWidget {
  const ServerHistoryScreen({super.key, required this.flow, required this.feeder});

  final ServerHistoryFlow flow;
  final FeederController feeder;

  @override
  State<ServerHistoryScreen> createState() => _ServerHistoryScreenState();
}

class _ServerHistoryScreenState extends State<ServerHistoryScreen> {
  @override
  void initState() {
    super.initState();
    unawaited(widget.flow.load());
  }

  @override
  void dispose() {
    widget.flow.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.flow,
    builder: (context, _) {
      final ServerHistoryState state = widget.flow.state;
      return Scaffold(
        appBar: AppBar(title: Text('${widget.feeder.state.known.name}: full history')),
        body: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // The pages come oldest first: the newest are on top once all are in.
            if (state.reading) const LinearProgressIndicator(semanticsLabel: 'Reading the history'),
            Expanded(
              child: HistoryTab(
                feeder: widget.feeder,
                entries: state.entries,
                empty: switch (state) {
                  ServerHistoryState(:final error?) => serverErrorWords(error),
                  ServerHistoryState(reading: true) => 'Reading the history from the server…',
                  _ => 'The server has no history of this feeder yet.',
                },
              ),
            ),
          ],
        ),
      );
    },
  );
}
