/// Changes to one file, run one after another: each starts when the last
/// has ended, so two never read the same contents and the later one's write
/// is the one that stays.
class OneAtATime {
  Future<void> _last = Future.value();

  /// Run `change` after every change already queued. Its own failure is
  /// its caller's to hear; the next change still runs.
  Future<void> run(Future<void> Function() change) {
    final Future<void> done = _last.then((_) => change());
    _last = done.catchError((Object _) {});
    return done;
  }
}
