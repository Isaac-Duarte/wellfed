import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/home/feeder_card.dart' show skewWords;
import 'package:wellfed/src/roles.g.dart';
import 'package:wellfed/src/rust/api/model.dart' show ClientRole, Skew;
import 'package:wellfed/state/feeder/feeder_state.dart';

import '../support/fakes.dart';
import 'feeder_support.dart';

/// The app gates its controls on the feeder's own matrix, generated from
/// `wellfed_proto::access`; these check the gates read it.
void main() {
  FeederState holding(ClientRole? role) => FeederState(
    known: feederKitchen,
    connection: Connection.online,
    card: fakeCard(),
    cardFresh: true,
    role: role,
    skew: Skew.feederBehind,
  );

  test('a phone offers a feed when the feeder lets its role feed', () {
    for (final ClientRole role in ClientRole.values) {
      expect(holding(role).canFeed, mayAsk(role, Ask.feed), reason: '$role');
    }
    expect(holding(ClientRole.member).canFeed, isTrue);
    expect(holding(ClientRole.unknown).canFeed, isFalse, reason: 'a role a newer build added grants nothing');
    expect(holding(null).canFeed, isTrue, reason: 'a role no session has named yet is left to the feeder');
  });

  test('a phone invites only as the feeder lets its role', () {
    expect(holding(ClientRole.owner).invitable, [ClientRole.member, ClientRole.owner]);
    expect(holding(ClientRole.member).invitable, [ClientRole.member]);
    expect(holding(ClientRole.unknown).invitable, isEmpty, reason: 'a role a newer build added grants nothing');
    expect(holding(null).invitable, isEmpty, reason: 'no session has named the role');
    for (final ClientRole role in ClientRole.values) {
      expect(holding(role).may(Ask.reboot), mayAsk(role, Ask.reboot), reason: '$role');
    }
    expect(mayAsk(ClientRole.member, Ask.reboot), isFalse, reason: "a restart is an owner's");
  });

  test('only a role that may update the feeder is told to update it', () {
    expect(mayAsk(ClientRole.owner, Ask.offerUpdate), isTrue);
    expect(mayAsk(ClientRole.member, Ask.offerUpdate), isFalse);
    expect(skewWords(holding(ClientRole.owner)), 'Update the feeder: it runs older firmware than this app');
    expect(skewWords(holding(ClientRole.member)), 'The feeder runs older firmware: its owner can update it');
  });
}
