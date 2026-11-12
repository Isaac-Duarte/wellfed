import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/server_error.dart';
import 'package:wellfed/src/rust/api/server_views.dart';
import 'package:wellfed/state/server/records.dart';
import 'package:wellfed/state/server/server.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/icons.dart';

/// What a server error means to the owner, in a sentence.
String serverErrorWords(ServerError error) => switch (error.kind) {
  ServerErrorKind.address => 'That is not a server address. It looks like http://192.168.1.20:8080.',
  ServerErrorKind.redirected => 'The server sent the $deviceName elsewhere. Use this address instead: ${error.message}',
  ServerErrorKind.unreachable => 'Nothing answered at that address. Check the $deviceName is on the home network.',
  ServerErrorKind.interrupted => 'The connection broke before the server answered.',
  // A read or a login; a change that may have run gets its own words from
  // the screen that made it.
  ServerErrorKind.serverFailed => 'The server failed partway through. Try again in a while.',
  ServerErrorKind.badCredentials => 'The name or the password is wrong.',
  ServerErrorKind.rateLimited => switch (error.retryAfterSeconds) {
    final int seconds? => 'Too many tries. Try again in $seconds seconds.',
    null => 'Too many tries. Try again in a while.',
  },
  ServerErrorKind.tokenRevoked => 'The server no longer takes this $deviceName. Log in again.',
  ServerErrorKind.accountDisabled => 'This account has expired. An owner can extend it on the server.',
  ServerErrorKind.forbidden => 'Your account may not do that.',
  ServerErrorKind.notFound => 'The server no longer has that.',
  ServerErrorKind.invalid => 'The server did not take that: ${error.message}',
  ServerErrorKind.foodInUse => 'A feeder held this food, so it stays in the history.',
  ServerErrorKind.noCurrentFood => 'Set the food this feeder holds first.',
  ServerErrorKind.alreadyAdopted => 'The server already has this feeder.',
  ServerErrorKind.inviteExpired => 'The feeder dropped the invite. Try again.',
  ServerErrorKind.phoneKeyUnconfirmed => "The server couldn't reach the feeder to confirm this $deviceName. Try again.",
  ServerErrorKind.phoneKeyRefused => 'The server would not take this $deviceName as yours: ${error.message}',
  ServerErrorKind.feederOffline => "The server can't reach this feeder right now.",
  ServerErrorKind.outcomeUnknown => 'The feeder did not answer in time. Check before you try again.',
  ServerErrorKind.keyReused || ServerErrorKind.refused => 'The server refused: ${error.message}',
  ServerErrorKind.protocol => 'That does not answer as a Wellfed server of this version.',
  ServerErrorKind.closed => 'The server connection is closed.',
  ServerErrorKind.feederBusy => 'This feeder is still being put on the server. Check the feeder list.',
  ServerErrorKind.plainHttp =>
    'Plain http:// would send your password unencrypted beyond the home network. '
        "Use the server's https:// address, or its home-network address like 192.168.1.20:8080.",
};

/// What a records screen's last read or change ran into, in a sentence. A
/// change that may have run says so: the list shows what the server holds.
String recordProblemWords(RecordProblem problem) => problem.mayHaveRun
    ? 'The server may or may not have taken that. What it holds now is shown here; check it before you try again.'
    : serverErrorWords(problem.error);

/// What an account may do, in a word.
String roleWords(ServerRole role) => switch (role) {
  ServerRole.owner => 'Owner',
  ServerRole.member => 'Member',
  ServerRole.viewer => 'Viewer',
  ServerRole.unknown => 'A role this app does not know',
};

/// A plain page of the server's screens: a back button and a title.
class ServerPage extends StatelessWidget {
  const ServerPage({super.key, required this.title, required this.children, this.action});

  final String title;
  final List<Widget> children;

  /// A button for the bottom right, to add something.
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Scaffold(
      appBar: AppBar(
        toolbarHeight: 64,
        automaticallyImplyLeading: false,
        titleSpacing: 4,
        leading: IconButton(
          tooltip: 'Back',
          onPressed: () => Navigator.pop(context),
          icon: StrokeIcon(Glyph.back, color: c.text),
        ),
        title: Text(
          title,
          style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
        ),
      ),
      floatingActionButton: action,
      body: ListView(padding: const EdgeInsets.all(Space.s4), children: children),
    );
  }
}

/// A line of muted text under a heading.
class Muted extends StatelessWidget {
  const Muted(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) => Text(
    text,
    style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: context.colors.textMuted),
  );
}

/// What the home screen says of a device the server lists that this app
/// does not serve as a feeder.
String otherDeviceWords(OtherDevice other) => switch (other.device) {
  ServerDevice.newer => '${other.name} is a kind of device this app does not know yet. Update the app to use it.',
  ServerDevice.otherKind =>
    "${other.name} is on the home server, but this app doesn't control that kind of device yet.",
  // Never an other device: a feeder is listed as one.
  ServerDevice.feeder => '${other.name} is a feeder.',
};
