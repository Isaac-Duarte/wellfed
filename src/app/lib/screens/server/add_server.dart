import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/server/server_words.dart';
import 'package:wellfed/state/server/add_server.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
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

  void _changed() {
    // Logged in: the account screen is where the owner goes next.
    if (_flow.state.stage is AddServerDone && mounted) Navigator.pop(context, true);
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
          const Muted(
            'A Wellfed server on your network keeps the long history, reaches feeders '
            'away from this phone, and lets the household share them. The phone still '
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
                subtitle: Text(found.url, style: TextStyle(color: c.textMuted)),
                selected: _url.text == found.url,
                onTap: () => _url.text = found.url,
              ),
            const SizedBox(height: Space.s3),
          ],
          LabeledField(
            label: 'Address',
            hint: 'As the server prints it, like http://192.168.1.20:8080',
            child: TextField(controller: _url, keyboardType: TextInputType.url, autocorrect: false, enabled: !sending),
          ),
          const SizedBox(height: Space.s3),
          LabeledField(
            label: 'Name',
            child: TextField(controller: _name, autocorrect: false, enabled: !sending),
          ),
          const SizedBox(height: Space.s3),
          LabeledField(
            label: 'Password',
            hint: 'Used once to sign this phone in. It is not kept.',
            child: TextField(controller: _password, obscureText: true, autocorrect: false, enabled: !sending),
          ),
          const SizedBox(height: Space.s4),
          if (state.stage case AddServerFailed(:final error)) ...[
            Notice(tone: Tone.fault, text: serverErrorWords(error)),
            const SizedBox(height: Space.s3),
          ],
          FilledButton(onPressed: filled && !sending ? _logIn : null, child: Text(sending ? 'Logging in' : 'Log in')),
        ],
      );
    },
  );
}
