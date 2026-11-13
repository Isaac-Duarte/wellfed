import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:nsd/nsd.dart' as nsd;

import 'package:wellfed/platform/host.dart';
import 'package:wellfed/src/rust/api/discovery.dart';

export 'package:wellfed/src/rust/api/discovery.dart' show FoundFeeder;

/// Feeders on the LAN, `_wellfed._tcp`, each with its identity in the TXT
/// `id`. Android browses through NsdManager, which holds the multicast lock
/// a phone needs. A desktop browses through the bridge's mDNS, the same as
/// the CLI's, and uses no `nsd` at all, which has no Linux side.
class Discovery {
  /// The NSD service feeders advertise.
  static const serviceType = '_wellfed._tcp';

  /// Every feeder found until the returned stream is cancelled.
  static Stream<FoundFeeder> browse() => onDesktopHost ? _bridge() : _nsd(serviceType, _feeder);

  static Stream<FoundFeeder> _bridge() {
    late final StreamController<FoundFeeder> out;
    FeederBrowse? browse;
    StreamSubscription<FoundFeeder>? heard;

    out = StreamController<FoundFeeder>(
      onListen: () {
        try {
          final started = FeederBrowse();
          browse = started;
          heard = started.run().listen(
            out.add,
            onError: out.addError,
            // Ended by itself: free the handle now rather than at finalization.
            onDone: () {
              started.dispose();
              browse = null;
              unawaited(out.close());
            },
          );
        } on Object catch (error) {
          out.addError(error);
        }
      },
      onCancel: () async {
        // Stopping is what ends the browse's thread and its daemon; the
        // stream it feeds closes after.
        browse?.stop();
        await heard?.cancel();
        browse?.dispose();
        await out.close();
      },
    );
    return out.stream;
  }

  /// How long Android may take to say discovery stopped. Only its callback
  /// ends the wait, and a catch-up round waits on it, so one that never comes
  /// must not stall every later round. The plugin lets go of its multicast
  /// lock as the stop goes in, before this wait.
  static const _stopWait = Duration(seconds: 3);

  static Stream<T> _nsd<T>(String type, T? Function(nsd.Service) read) {
    late final StreamController<T> out;
    nsd.Discovery? discovery;
    var cancelled = false;

    Future<void> stop() async {
      final running = discovery;
      discovery = null;
      if (running != null) await nsd.stopDiscovery(running).timeout(_stopWait, onTimeout: () {});
    }

    out = StreamController<T>(
      onListen: () async {
        try {
          final nsd.Discovery started = await nsd.startDiscovery(type, ipLookupType: nsd.IpLookupType.v4);
          // Cancelled while starting: stop what just started.
          if (cancelled) {
            await nsd.stopDiscovery(started).timeout(_stopWait, onTimeout: () {});
            return;
          }
          discovery = started;
          started.addServiceListener((service, status) {
            final T? found = read(service);
            if (status == nsd.ServiceStatus.found && found != null && !out.isClosed) {
              out.add(found);
            }
          });
        } on Object catch (error) {
          if (!out.isClosed) out.addError(error);
        }
      },
      onCancel: () async {
        cancelled = true;
        await stop();
        await out.close();
      },
    );
    return out.stream;
  }

  /// Where the feeder with `identity` answers, within `wait`; `null` when
  /// it did not announce itself in time.
  static Future<String?> locate(String identity, Duration wait) async {
    final found = Completer<String?>();
    // Cancelling the subscription is what stops the browse, so it is
    // cancelled on every path, the timeout's included.
    final StreamSubscription<FoundFeeder> browsing = browse().listen(
      (feeder) {
        if (feeder.identity == identity && !found.isCompleted) {
          found.complete(feeder.address);
        }
      },
      onError: (Object _) {
        if (!found.isCompleted) found.complete(null);
      },
    );
    final timer = Timer(wait, () {
      if (!found.isCompleted) found.complete(null);
    });
    try {
      return await found.future;
    } finally {
      timer.cancel();
      await browsing.cancel();
    }
  }

  static FoundFeeder? _feeder(nsd.Service service) {
    final Uint8List? id = service.txt?['id'];
    final InternetAddress? ip = service.addresses?.where((a) => a.type == InternetAddressType.IPv4).firstOrNull;
    final int? port = service.port;
    if (id == null || ip == null || port == null) return null;
    return FoundFeeder(identity: utf8.decode(id), address: '${ip.address}:$port', name: service.name ?? 'Wellfed');
  }
}
