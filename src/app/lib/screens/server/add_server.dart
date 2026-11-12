import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/state/server/add_server.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/desktop.dart';
import 'package:wellfed/widgets/layout.dart';

/// Add the owner's server: its address, typed or heard on the LAN, and the
/// account's name and password. The password mints this phone's own token
/// and is kept nowhere.
class AddServerScreen extends StatefulWidget {
  const AddServerScreen({super.key, required this.newFlow});

  final AddServerFlow Function() newFlow;

  @override
  State<AddServerScreen> createState() => _AddServerScreenState();
}

class _AddServerScreenState extends State<AddServerScreen> {
  late final AddServerFlow _flow = widget.newFlow()..addListener(_changed);
  final _url = TextEditingController();
  final _name = TextEditingController();
  final _password = TextEditingController();

  /// This screen has popped itself: it is still mounted while it leaves.
  bool _left = false;

  void _changed() {
    // Logged in: the account screen is where the owner goes next, once.
    if (_flow.state.stage is! AddServerDone || !mounted || _left) return;
    _left = true;
    Navigator.pop(context, true);
  }

  @override
  void dispose() {
    _flow
      ..removeListener(_changed)
      ..dispose();
    _url.dispose();
    _name.dispose();
    _password.dispose();
    super.dispose();
  }

  /// An address typed with plain http, which the app takes only on this
  /// device itself.
  static bool _plain(String url) => url.trim().toLowerCase().startsWith('http://');

  void _logIn() => unawaited(_flow.logIn(url: _url.text, name: _name.text, password: _password.text));

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([_flow, _url, _name, _password]),
    builder: (context, _) {
      final ThemeTokens c = context.colors;
      final AddServerState state = _flow.state;
      final bool sending = state.stage is AddServerLoggingIn;
      final bool filled = _url.text.trim().isNotEmpty && _name.text.trim().isNotEmpty && _password.text.isNotEmpty;
      return ServerPage(
        title: 'Add a home server',
        children: [
          Muted(
            'A Wellfed server on your network keeps the long history, reaches feeders '
            'away from this $deviceName, and lets the household share them. The $deviceName still '
            'reaches its own feeders directly when the server is down.',
          ),
          const SizedBox(height: Space.s4),
          if (state.found.isNotEmpty) ...[
            const SectionLabel('On this network'),
            const SizedBox(height: Space.s2),
            for (final HeardServer found in state.found)
              ListTile(
                contentPadding: EdgeInsets.zero,
                title: Text(found.name, style: TextStyle(color: c.text)),
                subtitle: Text(switch (found.fingerprint) {
                  final String fingerprint? => '${found.url}\nKey $fingerprint',
                  null => found.url,
                }, style: TextStyle(color: c.textMuted)),
                selected: _url.text == found.url,
                onTap: () => _url.text = found.url,
              ),
            const SizedBox(height: Space.s3),
          ],
          LabeledField(
            label: 'Address',
            hint: 'As the server prints it, like https://192.168.1.20:8443',
            child: TextField(controller: _url, keyboardType: TextInputType.url, autocorrect: false, enabled: !sending),
          ),
          // Plain http shows the password to anyone listening, so the app
          // takes it only on this device itself.
          if (_plain(_url.text)) ...[
            const SizedBox(height: Space.s2),
            const Notice(
              tone: Tone.warning,
              lead: 'Not encrypted.',
              text: "Plain http:// works only on this device itself. Use the server's https:// address.",
            ),
          ],
          const SizedBox(height: Space.s3),
          LabeledField(
            label: 'Name',
            child: TextField(controller: _name, autocorrect: false, enabled: !sending),
          ),
          const SizedBox(height: Space.s3),
          LabeledField(
            label: 'Password',
            hint: 'Used once to sign this $deviceName in. It is not kept.',
            child: TextField(controller: _password, obscureText: true, autocorrect: false, enabled: !sending),
          ),
          const SizedBox(height: Space.s4),
          if (state.stage case AddServerFailed(:final error)) ...[
            Notice(tone: Tone.fault, text: serverErrorWords(error)),
            const SizedBox(height: Space.s3),
          ],
          if (state.stage is AddServerNotKept) ...[
            Notice(
              tone: Tone.fault,
              text:
                  "This $deviceName's storage couldn't keep the sign-in, so nothing was kept. "
                  'Try again; if this stays, restart the $deviceName.',
            ),
            const SizedBox(height: Space.s3),
          ],
          FilledButton(onPressed: filled && !sending ? _logIn : null, child: Text(sending ? 'Logging in' : 'Log in')),
        ],
      );
    },
  );
}
