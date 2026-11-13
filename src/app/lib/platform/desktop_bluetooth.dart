import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_reactive_ble/flutter_reactive_ble.dart' show DiscoveredDevice;
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart' show RustOpaqueInterface;

import 'package:wellfed/platform/setup_link.dart';
import 'package:wellfed/src/rust/api/bluetooth.dart';
import 'package:wellfed/src/rust/api/error.dart';

/// A computer's Bluetooth failed: the scan could not run, or the link to the
/// feeder broke. Not a [BridgeError], which a setup reads as the feeder's
/// answer: this is the radio's, as the phone plugin's own failures are.
class BluetoothFailure implements Exception {
  /// A failure the radio described as `message`.
  const BluetoothFailure(this.message);

  /// What the radio said.
  final String message;

  @override
  String toString() => message;
}

/// The radio half of BLE setup on a computer, through the bridge's
/// `btleplug`, the CLI's own: Android's plugin has no desktop side. It hands
/// the state machines the plugin's own types, so they run unchanged.
class DesktopBluetooth {
  /// The radio through the bridge, unless a test gives its own handles.
  const DesktopBluetooth({
    this.newScan = SetupScan.new,
    this.newWatch = PowerWatch.new,
    this.openLink = _openLink,
    this.openSettings = _openSettings,
  });

  /// How long a connection may take before it is taken as lost. The bridge
  /// bounds its own open at 75 s (opening the radio, listening again for a
  /// feeder the stack forgot, connecting, discovering its services and
  /// subscribing, each bounded), so its own error comes first; this only
  /// catches a bridge that never answers. A link it opens after this is
  /// hung up as it comes.
  static const openWait = Duration(seconds: 90);

  /// How long one write may take. The bridge bounds it at
  /// [SetupLink.connectTimeout]; the margin lets its error come first.
  static const writeWait = Duration(seconds: 20);

  /// Starts one scan.
  final SetupScan Function() newScan;

  /// Starts one watch of the adapter's power.
  final PowerWatch Function() newWatch;

  /// Connects to the feeder a scan heard as `id`.
  final Future<BluetoothLink> Function(String id) openLink;

  /// Shows the system's Bluetooth settings, where the owner turns it on.
  final Future<void> Function() openSettings;

  static Future<BluetoothLink> _openLink(String id) => openBluetoothLink(id: id);

  /// macOS and Windows open their Bluetooth settings by name; Linux desktops
  /// share no way to, so the screen's words are all there is.
  static Future<void> _openSettings() async {
    try {
      if (Platform.isMacOS) {
        await Process.run('open', ['x-apple.systempreferences:com.apple.BluetoothSettings']);
      } else if (Platform.isWindows) {
        await Process.run('explorer.exe', ['ms-settings:bluetooth']);
      }
    } on ProcessException {
      // Nothing to open: the owner turns it on from the system's own menus.
    }
  }

  /// Every sighting of a feeder advertising the setup service until
  /// cancelled, again on each round it is heard. A scan that fails ends the
  /// stream with a [BluetoothFailure].
  Stream<DiscoveredDevice> scan() => _handle<SetupScan, ScanEvent, DiscoveredDevice>(
    newScan,
    (scan) => scan.run(),
    (scan) => scan.stop(),
    (event, out) => switch (event) {
      ScanEvent_Heard(:final feeder) => out.add(_device(feeder)),
      ScanEvent_Failed(:final error) => out.addError(BluetoothFailure(error.message)),
    },
  );

  /// Whether this computer's Bluetooth is on, now and as it changes. Ends
  /// with nothing said when the radio can't be opened: the scan says why.
  Stream<BluetoothPower> power() => _handle<PowerWatch, AdapterPower, BluetoothPower>(
    newWatch,
    (watch) => watch.run(),
    (watch) => watch.stop(),
    (power, out) => out.add(power == AdapterPower.poweredOff ? BluetoothPower.off : BluetoothPower.on),
  );

  /// Connect to `device`, a feeder [scan] heard, with notifications on.
  Future<BleLink> open(DiscoveredDevice device) async {
    final Future<BluetoothLink> opening = openLink(device.id);
    final BluetoothLink link;
    try {
      link = await opening.timeout(openWait);
    } on BridgeError catch (error) {
      throw BluetoothFailure(error.message);
    } on TimeoutException {
      // Nobody will use a link that opens after this: hang it up as it
      // comes rather than when it is collected. The bridge hangs up only the
      // newest link to a feeder, so a retry's own link stays.
      unawaited(opening.then(_hangUp, onError: (Object _) {}));
      rethrow;
    }
    return DesktopLink(link);
  }

  static Future<void> _hangUp(BluetoothLink link) async {
    await link.close();
    link.dispose();
  }

  static DiscoveredDevice _device(HeardFeeder feeder) => DiscoveredDevice(
    id: feeder.id,
    name: feeder.name,
    serviceData: const {},
    manufacturerData: Uint8List(0),
    rssi: feeder.rssi,
    // The bridge reports only feeders advertising setup, and nothing reads it.
    serviceUuids: const [],
  );

  /// A bridge handle's stream, which runs until the handle is stopped: a
  /// cancel here stops it and frees it, and a stream that ends by itself
  /// frees it then.
  static Stream<T> _handle<H extends RustOpaqueInterface, E, T>(
    H Function() start,
    Stream<E> Function(H) run,
    void Function(H) stop,
    void Function(E event, StreamController<T> out) each,
  ) {
    late final StreamController<T> out;
    H? handle;
    StreamSubscription<E>? heard;

    out = StreamController<T>(
      onListen: () {
        try {
          final H started = start();
          handle = started;
          heard = run(started).listen(
            (event) {
              if (!out.isClosed) each(event, out);
            },
            onError: (Object error) {
              if (!out.isClosed) out.addError(error);
            },
            onDone: () {
              started.dispose();
              handle = null;
              unawaited(out.close());
            },
          );
        } on Object catch (error) {
          out.addError(error);
        }
      },
      onCancel: () async {
        final H? running = handle;
        handle = null;
        if (running != null) stop(running);
        await heard?.cancel();
        running?.dispose();
        // Not closed here: a stream that ended by itself gets here from its
        // own close, which would then wait on this.
      },
    );
    return out.stream;
  }
}

/// A feeder's control characteristic on a computer.
class DesktopLink implements BleLink {
  /// The bridge's `link`. Its notifications wait in Rust until the setup
  /// listens, so none sent before then is lost.
  DesktopLink(this._link) : notifications = _link.notifications().map<List<int>>((bytes) => bytes).asBroadcastStream();

  final BluetoothLink _link;
  bool _closed = false;

  @override
  final Stream<List<int>> notifications;

  @override
  Future<void> write(List<List<int>> pieces) async {
    for (final piece in pieces) {
      try {
        await _link.write(piece: piece).timeout(DesktopBluetooth.writeWait);
      } on BridgeError catch (error) {
        throw BluetoothFailure(error.message);
      }
    }
  }

  @override
  Future<void> close() async {
    if (_closed) return;
    _closed = true;
    await _link.close();
    _link.dispose();
  }
}
