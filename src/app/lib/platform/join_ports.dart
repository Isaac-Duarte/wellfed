import 'dart:typed_data';

import 'package:wellfed/platform/discovery.dart';
import 'package:wellfed/src/rust/api/join.dart' as rust show inviteFeeder;
import 'package:wellfed/src/rust/api/join.dart' show JoinAnswer, joinFeeder;
import 'package:wellfed/src/rust/api/keys.dart';
import 'package:wellfed/src/rust/api/store.dart';

/// What joining reaches outside Dart: Rust reads and redeems the invite, and
/// NSD finds the feeder. Behind this, so the screen is tested with neither.
abstract interface class JoinPorts {
  /// The identity (hex) of the feeder `text` invites to; `null` if it is not
  /// an invite.
  String? inviteFeeder(String text);

  /// Redeem `invite` at `address` under `label`, once.
  Future<JoinAnswer> join(String invite, String label, String address);

  /// Where the feeder `identity` (hex) answers, within `wait`.
  Future<String?> locate(String identity, Duration wait);
}

/// [JoinPorts] on Android, with this phone's key.
class AppJoinPorts implements JoinPorts {
  /// Ports signed by `_key`.
  const AppJoinPorts(this._key);

  final ClientKey _key;

  @override
  String? inviteFeeder(String text) {
    final Uint8List? feeder = rust.inviteFeeder(text: text);
    return feeder == null ? null : identityHex(identity: feeder);
  }

  @override
  Future<JoinAnswer> join(String invite, String label, String address) =>
      joinFeeder(key: _key, invite: invite, label: label, address: address);

  @override
  Future<String?> locate(String identity, Duration wait) => Discovery.locate(identity, wait);
}
