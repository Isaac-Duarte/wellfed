import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:nsd/nsd.dart' as nsd;

/// A feeder answering on the LAN: its identity (hex, from the TXT `id`)
/// and where its device API listens.
class FoundFeeder {
  /// A feeder as NSD heard it.
  const FoundFeeder({required this.identity, required this.address, required this.name});

  /// Its identity key, hex.
  final String identity;

  /// `ip:port`, IPv4.
  final String address;

  /// The service instance name the feeder announces.
  final String name;
}

/// Feeders on the LAN over Android's NsdManager: `_wellfed._tcp`, each
/// with its identity in the TXT `id`.
class Discovery {
  /// The NSD service feeders advertise.
  static const serviceType = '_wellfed._tcp';

  /// Every feeder found until the returned stream is cancelled.
  static Stream<FoundFeeder> browse() {
    late final StreamController<FoundFeeder> out;
    nsd.Discovery? discovery;
    var cancelled = false;

    Future<void> stop() async {
      final running = discovery;
      discovery = null;
      if (running != null) await nsd.stopDiscovery(running);
    }

    out = StreamController<FoundFeeder>(
      onListen: () async {
        try {
          final nsd.Discovery started = await nsd.startDiscovery(serviceType, ipLookupType: nsd.IpLookupType.v4);
          // Cancelled while starting: stop what just started.
          if (cancelled) {
            await nsd.stopDiscovery(started);
            return;
          }
          discovery = started;
          started.addServiceListener((service, status) {
            final FoundFeeder? found = _feeder(service);
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
    // Cancelling the subscription is what stops NsdManager's discovery, so
    // it is cancelled on every path, the timeout's included.
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
