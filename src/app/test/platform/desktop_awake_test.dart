import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/desktop_awake.dart';

/// A helper process that runs until its input closes or it is killed.
class _Helper implements Process {
  _Helper() {
    unawaited(
      _input.stream.drain<void>().then((_) {
        inputClosed = true;
        if (!_exit.isCompleted) _exit.complete(0);
      }),
    );
  }

  final _exit = Completer<int>();
  // ignore: close_sinks, the helper's stdin closes it, as the app's release does.
  final _input = StreamController<List<int>>();
  bool killed = false;
  bool inputClosed = false;

  /// A helper that is gone refuses to close its input.
  bool closeFails = false;

  @override
  // ignore: close_sinks, the code under test closes it.
  late final IOSink stdin = closeFails ? _FailingSink() : IOSink(_input.sink);

  @override
  Stream<List<int>> get stdout => const Stream.empty();

  @override
  Stream<List<int>> get stderr => const Stream.empty();

  @override
  Future<int> get exitCode => _exit.future;

  @override
  int get pid => 4242;

  @override
  bool kill([ProcessSignal signal = ProcessSignal.sigterm]) {
    killed = true;
    if (!_exit.isCompleted) _exit.complete(-15);
    return true;
  }
}

/// An input whose close fails, as a dead helper's pipe can.
class _FailingSink implements IOSink {
  @override
  Future<void> close() async => throw const FileSystemException('broken pipe');

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  late List<(String, List<String>)> started;
  late List<_Helper> helpers;

  Future<Process> start(String executable, List<String> arguments) async {
    started.add((executable, arguments));
    final helper = _Helper();
    helpers.add(helper);
    return helper;
  }

  setUp(() {
    started = [];
    helpers = [];
  });

  test('a helper that exits at once, as a refused systemd-inhibit does, is not a hold', () async {
    var lapsed = 0;
    final awake = DesktopAwake(
      start: (executable, arguments) async {
        final helper = await start(executable, arguments) as _Helper;
        helper.kill();
        return helper;
      },
      os: 'linux',
      onLapsed: () => lapsed++,
    );
    expect(await awake.apply('Installing'), isFalse);
    expect(lapsed, 0, reason: 'refused at once, it was never held');
  });

  test('a helper that ends mid-run says the hold gave way, once', () async {
    var lapsed = 0;
    final awake = DesktopAwake(start: start, os: 'linux', onLapsed: () => lapsed++);
    expect(await awake.apply('Installing'), isTrue);
    helpers.single.kill();
    await helpers.single.exitCode;
    await Future<void>.delayed(Duration.zero);
    expect(lapsed, 1);

    // A released hold is not a lapse.
    expect(await awake.apply('Installing'), isTrue);
    await awake.apply(null);
    await Future<void>.delayed(Duration.zero);
    expect(lapsed, 1);
  });

  test('a release while the helper is still starting ends it once it has started', () async {
    var lapsed = 0;
    final awake = DesktopAwake(start: start, os: 'linux', onLapsed: () => lapsed++);
    final Future<bool> holding = awake.apply('Installing');
    await Future<void>.delayed(Duration.zero);
    await awake.apply(null);

    expect(await holding, isFalse);
    expect(helpers.single.killed, isTrue);
    expect(helpers.single.inputClosed, isTrue);
    expect(lapsed, 0, reason: 'a release is no lapse');
  });

  test('a second hold while the first helper starts shares it', () async {
    final awake = DesktopAwake(start: start, os: 'linux');
    final List<bool> held = await Future.wait([awake.apply('Installing'), awake.apply('Updating')]);
    expect(held, [true, true]);
    expect(started, hasLength(1));
    await awake.apply(null);
  });

  test('a release, then a new hold, while the first helper starts holds with a fresh helper', () async {
    final awake = DesktopAwake(start: start, os: 'linux');
    final Future<bool> first = awake.apply('Installing');
    await Future<void>.delayed(Duration.zero);
    await awake.apply(null);
    final Future<bool> second = awake.apply('Updating');

    expect(await first, isFalse);
    expect(await second, isTrue);
    expect(started, hasLength(2));
    expect(helpers.first.killed, isTrue);
    expect(helpers.last.killed, isFalse);
    await awake.apply(null);
  });

  test('a helper whose input will not close is killed all the same', () async {
    final awake = DesktopAwake(
      start: (executable, arguments) async {
        final helper = _Helper()..closeFails = true;
        helpers.add(helper);
        return helper;
      },
      os: 'linux',
    );
    expect(await awake.apply('Installing'), isTrue);
    await awake.apply(null);
    expect(helpers.single.killed, isTrue);
  });

  test('Linux holds logind off idle and sleep until the run ends, then closes the pipe', () async {
    final awake = DesktopAwake(start: start, os: 'linux');
    expect(await awake.apply('Keep the feeder plugged in.'), isTrue);
    expect(started.single.$1, 'systemd-inhibit');
    expect(started.single.$2, containsAllInOrder(['--what=idle:sleep', '--why=Keep the feeder plugged in.', 'cat']));

    // A second run's words keep the one helper.
    expect(await awake.apply('Making Wellfed permanent.'), isTrue);
    expect(started, hasLength(1));

    expect(await awake.apply(null), isFalse);
    await helpers.single.exitCode;
    expect(helpers.single.inputClosed, isTrue, reason: '`cat` ends at the end of its input');
  });

  test('macOS asserts against idle sleep for as long as this process lives', () async {
    final awake = DesktopAwake(start: start, os: 'macos');
    expect(await awake.apply('Updating Kitchen'), isTrue);
    expect(started.single.$1, '/usr/bin/caffeinate');
    expect(started.single.$2, ['-i', '-w', '$pid']);
    await awake.apply(null);
    expect(helpers.single.killed, isTrue);
  });

  test('a computer with no helper says it is not held, and the run goes on', () async {
    final awake = DesktopAwake(
      start: (executable, arguments) async => throw const ProcessException('systemd-inhibit', [], 'not found'),
      os: 'linux',
    );
    expect(await awake.apply('Keep the feeder plugged in.'), isFalse);
    expect(await awake.apply(null), isFalse);
  });

  test('a helper that ends by itself is started again by the next hold', () async {
    final awake = DesktopAwake(start: start, os: 'linux');
    await awake.apply('Installing');
    helpers.single.kill();
    await helpers.single.exitCode;
    await Future<void>.delayed(Duration.zero);

    expect(await awake.apply('Installing'), isTrue);
    expect(started, hasLength(2));
  });
}
