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

/// An owner's server answering on the LAN: the URL to log in at, and the
/// name it announces.
class FoundServer {
  /// A server as NSD heard it.
  const FoundServer({required this.url, required this.name});

  /// `http://ip:port`, IPv4: what mDNS gives is the LAN's own address.
  final String url;

  /// The service instance name, the server's label.
  final String name;
}

/// Feeders on the LAN over Android's NsdManager: `_wellfed._tcp`, each
/// with its identity in the TXT `id`; and owner's servers,
/// `_wellfed-server._tcp`.
class Discovery {
  /// The NSD service feeders advertise.
  static const serviceType = '_wellfed._tcp';

  /// The NSD service an owner's server advertises.
  static const serverType = '_wellfed-server._tcp';

  /// Every feeder found until the returned stream is cancelled.
  static Stream<FoundFeeder> browse() => _browse(serviceType, _feeder);

  /// Every owner's server found until the returned stream is cancelled.
  static Stream<FoundServer> browseServers() => _browse(serverType, _server);

  static Stream<T> _browse<T>(String type, T? Function(nsd.Service) read) {
    late final StreamController<T> out;
    nsd.Discovery? discovery;
    var cancelled = false;

    Future<void> stop() async {
      final running = discovery;
      discovery = null;
      if (running != null) await nsd.stopDiscovery(running);
    }

    out = StreamController<T>(
      onListen: () async {
        try {
          final nsd.Discovery started = await nsd.startDiscovery(type, ipLookupType: nsd.IpLookupType.v4);
          // Cancelled while starting: stop what just started.
          if (cancelled) {
            await nsd.stopDiscovery(started);
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

  static FoundServer? _server(nsd.Service service) {
    final InternetAddress? ip = service.addresses?.where((a) => a.type == InternetAddressType.IPv4).firstOrNull;
    final int? port = service.port;
    if (ip == null || port == null) return null;
    return FoundServer(url: 'http://${ip.address}:$port', name: service.name ?? 'Home server');
  }

  static FoundFeeder? _feeder(nsd.Service service) {
    final Uint8List? id = service.txt?['id'];
    final InternetAddress? ip = service.addresses?.where((a) => a.type == InternetAddressType.IPv4).firstOrNull;
    final int? port = service.port;
    if (id == null || ip == null || port == null) return null;
    return FoundFeeder(identity: utf8.decode(id), address: '${ip.address}:$port', name: service.name ?? 'Wellfed');
  }
}
