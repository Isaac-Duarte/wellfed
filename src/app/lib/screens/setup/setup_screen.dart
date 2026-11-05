import 'dart:async';

import 'package:flutter/material.dart';
import 'package:wellfed/screens/feeder/lights_help.dart';
import 'package:wellfed/screens/setup/name_step.dart';
import 'package:wellfed/screens/setup/pairing_view.dart';
import 'package:wellfed/state/pairing/pairing.dart';
import 'package:wellfed/state/setup/setup.dart';
import 'package:wellfed/state/wifi/networks.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/cards.dart';
import 'package:wellfed/widgets/feeder_drawing.dart';
import 'package:wellfed/widgets/icons.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/network_picker.dart';

/// The control panel's band of the straight-on drawing, in its units: the
/// buttons and lights, without the hopper above or the bowl below.
const _panelBand = Rect.fromLTRB(0, 0.39, 1, 0.65);

/// BLE setup, from the panel to the code. [SetupFlow] holds its steps.
class SetupScreen extends StatefulWidget {
  const SetupScreen({super.key, required this.newFlow});

  /// The controller of this screen's one setup.
  final SetupFlow Function() newFlow;

  @override
  State<SetupScreen> createState() => _SetupScreenState();
}

class _SetupScreenState extends State<SetupScreen> {
  late final SetupFlow _flow = widget.newFlow();

  @override
  void initState() {
    super.initState();
    unawaited(_flow.load());
  }

  @override
  void dispose() {
    _flow.dispose();
    super.dispose();
  }

  Future<void> _grant() async {
    final bool granted = await _flow.grant();
    if (!granted && mounted) await _bluetoothOff();
  }

  Future<void> _bluetoothOff() => showDialog<void>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('Nearby devices is off'),
      content: const Text(
        'Setup talks to the feeder over Bluetooth. Allow "Nearby '
        'devices" for Wellfed in Android settings, then try again.',
      ),
      actions: [
        TextButton(onPressed: () => unawaited(_flow.openSettings()), child: const Text('Open settings')),
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('OK')),
      ],
    ),
  );

  Future<void> _pair({bool again = false}) async {
    final bool paired = await _flow.pair(again: again);
    if (!paired && mounted) await _bluetoothOff();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _flow,
    builder: (context, _) => switch (_flow.state) {
      SetupPermissions() => _Permissions(onContinue: () => unawaited(_grant())),
      SetupMode() => StepScaffold(
        stepLabel: 'Step 1 of 4',
        progress: 0.25,
        leading: Leading.close,
        title: 'Put the feeder in setup mode',
        content: [
          const FeederDrawing(
            view: FeederView.panel,
            crop: _panelBand,
            height: 130,
            highlights: {'button_lock', 'button_mic'},
            label: "Drawing of the feeder's control panel",
            callouts: [
              Callout(anchor: 'button_lock', side: CalloutSide.below, lines: ['Lock']),
              Callout(anchor: 'button_mic', side: CalloutSide.below, lines: ['Mic']),
            ],
          ),
          const SizedBox(height: Space.s4),
          const NumberedSteps([
            'Plug the feeder in. If the lock light is on, hold **Lock** for 5 seconds to unlock the panel.',
            'Hold **Lock** and **Mic** together for 5 seconds.',
            'The Wi-Fi light blinks slowly, about once a second. Setup mode lasts 15 minutes.',
          ]),
          Align(
            alignment: Alignment.centerLeft,
            child: TextButton(
              onPressed: () =>
                  unawaited(Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => const LightsHelp()))),
              child: const Text("The light isn't blinking"),
            ),
          ),
        ],
        actions: [FilledButton(onPressed: _flow.toWifi, child: const Text('The Wi-Fi light is blinking'))],
      ),
      SetupWifi(:final WifiCredentials? wifi) => WifiStep(
        initial: wifi,
        currentNetwork: _flow.currentNetwork,
        scanNetworks: _flow.scanNetworks,
        stepLabel: 'Step 2 of 4',
        progress: 0.5,
        onBack: _flow.toSetupMode,
        onDone: _flow.wifiGiven,
      ),
      SetupNames(:final String feederName) => NameStep(
        initial: _flow.label,
        feeder: feederName,
        suggest: _flow.phoneModel,
        stepLabel: 'Step 3 of 4',
        progress: 0.75,
        onBack: _flow.toWifi,
        action: 'Find the feeder',
        onDone: (label, feeder) async {
          final String? failed = await _flow.named(label, feeder);
          if (failed == null) unawaited(_pair());
          return failed;
        },
      ),
      SetupPairing(:final PairingFlow pairing) => PairingView(
        flow: pairing,
        ble: true,
        onRetry: () => unawaited(_pair(again: true)),
        onNewCode: () => unawaited(_pair(again: true)),
      ),
    },
  );
}

/// What setup will ask Android for, before it asks.
class _Permissions extends StatelessWidget {
  const _Permissions({required this.onContinue});

  final VoidCallback onContinue;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return StepScaffold(
      title: 'Two permissions, used only on your network',
      content: const [
        _PermissionRow(
          Glyph.bluetooth,
          'Nearby devices',
          'Bluetooth, only while you set a feeder up. A feeder turns '
              'Bluetooth off once setup finishes.',
        ),
        SizedBox(height: 20),
        _PermissionRow(
          Glyph.wifi,
          'Wi-Fi network',
          'To find your feeders on the network and to fill in your Wi-Fi '
              'name during setup.',
        ),
      ],
      actions: [
        Text(
          'Nothing you do in Wellfed leaves your home network unless you '
          'add a server yourself.',
          style: TextStyle(fontSize: Sizes.sm, height: 1.5, color: c.textMuted),
        ),
        FilledButton(onPressed: onContinue, child: const Text('Continue')),
      ],
    );
  }
}

/// The network the feeder joins after setup, picked from what this phone
/// hears. The phone's current network is chosen first when location is
/// allowed; Android needs it to read the name and list networks, and nothing
/// else uses it.
class WifiStep extends StatefulWidget {
  const WifiStep({
    super.key,
    required this.initial,
    required this.currentNetwork,
    required this.scanNetworks,
    required this.onBack,
    required this.onDone,
    this.stepLabel,
    this.progress,
    this.mayKeep = true,
  });

  final WifiCredentials? initial;

  /// The Wi-Fi this phone is on, when Android will say.
  final Future<String?> Function() currentNetwork;

  /// The networks this phone hears.
  final Future<NetworkScan> Function() scanNetworks;
  final VoidCallback onBack;
  final ValueChanged<WifiCredentials?> onDone;
  final String? stepLabel;
  final double? progress;

  /// A feeder may keep the Wi-Fi it has, unless it has none yet.
  final bool mayKeep;

  @override
  State<WifiStep> createState() => _WifiStepState();
}

class _WifiStepState extends State<WifiStep> {
  late final _ssid = TextEditingController(text: widget.initial?.ssid);
  late final _password = TextEditingController(text: widget.initial?.passphrase);
  bool _hidden = true;

  @override
  void dispose() {
    _ssid.dispose();
    _password.dispose();
    super.dispose();
  }

  bool get _valid => _ssid.text.isNotEmpty && _password.text.length >= 8 && _password.text.length <= 63;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return StepScaffold(
      stepLabel: widget.stepLabel,
      progress: widget.progress,
      onLeading: widget.onBack,
      title: 'Connect it to your Wi-Fi',
      content: [
        LabeledField(
          label: 'Network',
          child: NetworkPicker(
            controller: _ssid,
            scan: widget.scanNetworks,
            current: widget.currentNetwork,
            onChanged: () => setState(() {}),
          ),
        ),
        const SizedBox(height: 18),
        LabeledField(
          label: 'Password',
          hint: "8 to 63 characters. The feeder keeps it; this phone doesn't.",
          child: TextField(
            controller: _password,
            obscureText: _hidden,
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
      ],
      actions: [
        FilledButton(
          onPressed: _valid ? () => widget.onDone(WifiCredentials(ssid: _ssid.text, passphrase: _password.text)) : null,
          child: const Text('Continue'),
        ),
        // A feeder already on the network only needs a new owner.
        if (widget.mayKeep)
          TextButton(onPressed: () => widget.onDone(null), child: const Text("Keep the feeder's Wi-Fi")),
      ],
    );
  }
}

class _PermissionRow extends StatelessWidget {
  const _PermissionRow(this.glyph, this.title, this.body);

  final Glyph glyph;
  final String title;
  final String body;

  @override
  Widget build(BuildContext context) {
    final ThemeTokens c = context.colors;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        IconTile(glyph, size: 44),
        const SizedBox(width: Space.s4),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                title,
                style: TextStyle(fontSize: Sizes.md, fontWeight: Fonts.semibold, color: c.text),
              ),
              const SizedBox(height: Space.s1),
              Text(
                body,
                style: TextStyle(fontSize: Sizes.base, height: 1.5, color: c.textMuted),
              ),
            ],
          ),
        ),
      ],
    );
  }
}
