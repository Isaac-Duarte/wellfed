// Generated from wellfed_proto::access by src/app/rust/tests/protocol.rs; do not edit.

import 'package:wellfed/src/rust/api/model.dart' show ClientRole;

/// Each op the app may send, and each statistics section.
enum Ask {
  status,
  getSettings,
  putSettings,
  putWifi,
  setLock,
  play,
  timeSync,
  createInvite,
  createBoundInvite,
  listClients,
  revokeClient,
  feed,
  reboot,
  cancelFeed,
  offerUpdate,
  updateChunk,
  updateState,
  cancelUpdate,
  enrollUpdateKey,
  updateKeyStatement,
  getTime,
  putTime,
  tailLog,
  feedTrace,
  getCalibration,
  putCalibration,
  readBank1,
  getPlans,
  putPlans,
  resetDesiccant,
  getCall,
  startCall,
  callChunk,
  deleteCall,
  playCall,
  skipMeal,
  unskipMeal,
  getAlerts,
  statsCounters,
  statsMemory,
  statsFlash,
  statsPower,
  statsFood,
  statsFeeds,
  statsPortions,
}

/// Whether a client holding [role] may send [ask], by the feeder's own
/// rules: its matrix first, then for an invite the role it grants, which
/// pairing checks next; the invite rows ask about inviting a member. A
/// role a newer build added may send nothing.
bool mayAsk(ClientRole role, Ask ask) => _senders(ask).contains(role);

const Set<ClientRole> _owner = {ClientRole.owner};

const Set<ClientRole> _ownerMember = {ClientRole.owner, ClientRole.member};

const Set<ClientRole> _ownerMemberServer = {ClientRole.owner, ClientRole.member, ClientRole.server};

Set<ClientRole> _senders(Ask ask) => switch (ask) {
  Ask.status => _ownerMemberServer,
  Ask.getSettings => _ownerMemberServer,
  Ask.putSettings => _ownerMemberServer,
  Ask.putWifi => _owner,
  Ask.setLock => _ownerMemberServer,
  Ask.play => _ownerMemberServer,
  Ask.timeSync => _ownerMemberServer,
  Ask.createInvite => _ownerMember,
  Ask.createBoundInvite => _ownerMember,
  Ask.listClients => _ownerMemberServer,
  Ask.revokeClient => _owner,
  Ask.feed => _ownerMemberServer,
  Ask.reboot => _owner,
  Ask.cancelFeed => _ownerMemberServer,
  Ask.offerUpdate => _owner,
  Ask.updateChunk => _owner,
  Ask.updateState => _owner,
  Ask.cancelUpdate => _owner,
  Ask.enrollUpdateKey => _owner,
  Ask.updateKeyStatement => _owner,
  Ask.getTime => _ownerMemberServer,
  Ask.putTime => _owner,
  Ask.tailLog => _owner,
  Ask.feedTrace => _ownerMemberServer,
  Ask.getCalibration => _ownerMemberServer,
  Ask.putCalibration => _ownerMember,
  Ask.readBank1 => _owner,
  Ask.getPlans => _ownerMemberServer,
  Ask.putPlans => _ownerMemberServer,
  Ask.resetDesiccant => _ownerMemberServer,
  Ask.getCall => _ownerMemberServer,
  Ask.startCall => _owner,
  Ask.callChunk => _owner,
  Ask.deleteCall => _owner,
  Ask.playCall => _ownerMemberServer,
  Ask.skipMeal => _ownerMemberServer,
  Ask.unskipMeal => _ownerMemberServer,
  Ask.getAlerts => _ownerMemberServer,
  Ask.statsCounters => _ownerMemberServer,
  Ask.statsMemory => _owner,
  Ask.statsFlash => _ownerMemberServer,
  Ask.statsPower => _ownerMemberServer,
  Ask.statsFood => _ownerMemberServer,
  Ask.statsFeeds => _ownerMemberServer,
  Ask.statsPortions => _ownerMemberServer,
};
