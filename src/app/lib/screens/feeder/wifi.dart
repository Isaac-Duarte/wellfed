import 'package:flutter/material.dart';
import 'package:wellfed/screens/feeder/settings_tab.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/state/feeder/feeder_state.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/network_picker.dart';
import 'package:wellfed/widgets/replies.dart';

/// Change the feeder's Wi-Fi (10.4). The credentials are stored for its next
/// join, and it rejoins only on a restart, so saving them is followed by one.
/// Each goes out once. A save whose answer never came is not followed by
/// anything: the owner looks first. The feeder never reports the network's
/// name, so this says only how it is doing.
class WifiScreen extends StatefulWidget {
  const WifiScreen({super.key, required this.feeder});

  final FeederController feeder;

  @override
  State<WifiScreen> createState() => _WifiScreenState();
}

class _WifiScreenState extends State<WifiScreen> {
  final _network = TextEditingController();
  final _password = TextEditingController();
  bool _asking = false;
  bool _hidden = true;

  @override
  void dispose() {
    _network.dispose();
    _password.dispose();
    super.dispose();
  }

  Future<void> _switch() async {
    final FeederController feeder = widget.feeder;
    final int? base = feeder.state.card?.wifiGeneration;
    if (base == null) return;

    setState(() => _asking = true);
    final FeederReply saved = await feeder.ask(
      FeederRequest.putWifi(base: base, ssid: _network.text, passphrase: _password.text),
    );
    if (!mounted) return;
    if (!showProblem(context, saved)) {
      setState(() => _asking = false);
      return;
    }

    final FeederReply restart = await feeder.ask(const FeederRequest.reboot());
    if (!mounted) return;
    setState(() => _asking = false);
    final String? problem = replyProblem(restart);
    final ScaffoldMessengerState messenger = ScaffoldMessenger.of(context);
    if (problem != null) {
      messenger.showSnackBar(
        SnackBar(
          content: Text(
            'The network is saved, but the restart: $problem Restart it to '
            'join.',
          ),
        ),
      );
      return;
    }
    messenger.showSnackBar(SnackBar(content: Text('${feeder.state.known.name} is restarting to join.')));
    Navigator.pop(context);
  }

  /// Forget the network, once. It stays on the network it has until a restart,
  /// and after one only Bluetooth setup brings it back.
  Future<void> _forget() async {
    final FeederController feeder = widget.feeder;
    final int? base = feeder.state.card?.wifiGeneration;
    if (base == null) return;

    final bool? sure = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Forget this network?'),
        content: const Text(
          'After its next restart the feeder stays off Wi-Fi until you set it '
          'up again over Bluetooth. Meals keep running either way.',
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('Forget')),
        ],
      ),
    );
    if (sure != true || !mounted) return;

    setState(() => _asking = true);
    final FeederReply reply = await feeder.ask(FeederRequest.forgetWifi(base: base));
    if (!mounted) return;
    setState(() => _asking = false);
    showProblem(context, reply);
  }

  bool get _valid => _network.text.isNotEmpty && _password.text.length >= 8 && _password.text.length <= 63;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    final FeederController feeder = widget.feeder;
    final bool online = feeder.state.connection == Connection.online;
    final int? rssi = feeder.state.card?.rssi;
    final String status = !online
        ? "Can't reach it right now"
        : switch (rssi) {
            null => 'Online',
            >= -60 => 'Online · strong signal',
            >= -72 => 'Online · fair signal',
            _ => 'Online · weak signal',
          };
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
          'Wi-Fi',
          style: TextStyle(fontSize: 20, fontWeight: Fonts.semibold, color: c.text),
        ),
      ),
      body: ListView(
        padding: const EdgeInsets.all(Space.s4),
        children: [
          Panel(
            padding: const EdgeInsets.all(Space.s4),
            child: Row(
              children: [
                IconTile(online ? Glyph.wifi : Glyph.wifiOff, muted: !online),
                const SizedBox(width: Space.s4),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        status,
                        style: TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: c.text),
                      ),
                      const SizedBox(height: 2),
                      Text(
                        "The feeder doesn't report the network's name.",
                        style: TextStyle(fontSize: Sizes.sm, color: c.textMuted),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: Space.s5),
          SettingsSection(
            'Switch network',
            last: true,
            Padding(
              padding: const EdgeInsets.all(Space.s4),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  LabeledField(
                    label: 'Network',
                    child: NetworkPicker(
                      controller: _network,
                      scan: feeder.scanNetworks,
                      current: feeder.currentNetwork,
                      onChanged: () => setState(() {}),
                    ),
                  ),
                  const SizedBox(height: Space.s4),
                  LabeledField(
                    label: 'Password',
                    hint: '8 to 63 characters.',
                    child: TextField(
                      controller: _password,
                      obscureText: _hidden,
                      autocorrect: false,
                      enableSuggestions: false,
                      decoration: InputDecoration(
                        suffixIcon: IconButton(
                          tooltip: _hidden ? 'Show password' : 'Hide password',
                          icon: StrokeIcon(_hidden ? Glyph.eye : Glyph.eyeOff, size: 20, color: c.textMuted),
                          onPressed: () => setState(() => _hidden = !_hidden),
                        ),
                      ),
                      onChanged: (_) => setState(() {}),
                    ),
                  ),
                  const SizedBox(height: Space.s4),
                  const Notice(
                    tone: Tone.neutral,
                    glyph: Glyph.info,
                    text:
                        "It restarts to join. If it can't, set it up again over Bluetooth. Meals keep running "
                        'either way.',
                  ),
                  const SizedBox(height: Space.s4),
                  FilledButton(onPressed: _asking || !_valid ? null : _switch, child: const Text('Switch network')),
                ],
              ),
            ),
          ),
          const SizedBox(height: Space.s5),
          Center(
            child: TextButton(
              style: TextButton.styleFrom(foregroundColor: c.fault),
              onPressed: _asking ? null : _forget,
              child: const Text('Forget this network'),
            ),
          ),
        ],
      ),
    );
  }
}
