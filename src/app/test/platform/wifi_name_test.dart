import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/platform/wifi_name.dart';

/// `nmcli -g IN-USE,SSID device wifi list ifname wlan0 --rescan no`, with a
/// network whose name holds a colon.
const _nmcli = r'''
:Maple St
*:PL-AF103-1A2B
:Cafe\: Guest
''';

const _nmcliEscaped = r'''
*:Cafe\: Guest
''';

/// `iw dev wlan0 link`.
const _iw = '''
Connected to 58:6d:8f:12:34:56 (on wlan0)
\tSSID: PL-AF103-1A2B
\tfreq: 2462
\tsignal: -41 dBm
''';

/// `netsh wlan show interfaces` with two adapters, as Windows 11 prints it.
const _netsh = '''

There are 2 interfaces on the system:

    Name                   : Wi-Fi
    Description            : Intel(R) Wi-Fi 6 AX201 160MHz
    GUID                   : 3f1c2b4a-0000-4000-8000-000000000001
    Physical address       : aa:bb:cc:dd:ee:01
    Interface type         : Primary
    State                  : connected
    SSID                   : Maple St
    AP BSSID               : 58:6d:8f:00:00:01
    Band                   : 5 GHz

    Name                   : Wi-Fi 2
    Description            : USB Wi-Fi adapter
    GUID                   : 3f1c2b4a-0000-4000-8000-000000000002
    Physical address       : aa:bb:cc:dd:ee:02
    Interface type         : Primary
    State                  : connected
    SSID                   : PL-AF103-1A2B
    AP BSSID               : 58:6d:8f:12:34:56

    Hosted network status  : Not available
''';

/// The same in French: Windows translates the labels, not `SSID`.
const _netshFrench = '''

Il existe 1 interface sur le système :

    Nom                    : Wi-Fi
    Description            : Intel(R) Wi-Fi 6 AX201 160MHz
    État                   : connecté
    SSID                   : PL-AF103-1A2B
''';

/// Windows 11 24H2 with location off.
const _netshNoLocation = '''
Network shell commands need location permission to access WLAN information. Turn on Location services on the Location page in Privacy & security settings.
''';

void main() {
  group('NetworkManager', () {
    test('refuses a nearby name that forges an in-use line', () {
      expect(WifiNames.nmcliName(' :Evil\n*:PL-AF103-ab12\n*:Cafe\n'), isNull);
    });
    test('refuses a name with a control character', () {
      expect(WifiNames.nmcliName('*:PL-AF103-ab12\x07\n'), isNull);
    });
    test('names the access point in use', () => expect(WifiNames.nmcliName(_nmcli), 'PL-AF103-1A2B'));
    test('unescapes a name', () => expect(WifiNames.nmcliName(_nmcliEscaped), 'Cafe: Guest'));
    test('names none when nothing is in use, or nmcli failed', () {
      expect(WifiNames.nmcliName(':Maple St\n'), isNull);
      expect(WifiNames.nmcliName(null), isNull);
    });
  });

  group('iw', () {
    test('names the linked network', () => expect(WifiNames.iwName(_iw), 'PL-AF103-1A2B'));
    test('names none when not connected', () => expect(WifiNames.iwName('Not connected.\n'), isNull));
    test('refuses a name with a control character', () {
      expect(WifiNames.iwName('Connected to 58:6d:8f:12:34:56 (on wlan0)\n\tSSID: PL-AF103-\x1bab\n'), isNull);
    });
  });

  group('netsh', () {
    test('names the asked interface, not the first', () {
      expect(WifiNames.netshName(_netsh, 'Wi-Fi 2'), 'PL-AF103-1A2B');
      expect(WifiNames.netshName(_netsh, 'Wi-Fi'), 'Maple St');
    });
    test('refuses a name with a control character', () {
      expect(WifiNames.netshName('    Name : Wi-Fi\n    SSID : PL-AF103-\x01ab\n', 'Wi-Fi'), isNull);
    });
    test('reads translated labels', () => expect(WifiNames.netshName(_netshFrench, 'Wi-Fi'), 'PL-AF103-1A2B'));
    test('names none for an interface it does not list, or with location off', () {
      expect(WifiNames.netshName(_netsh, 'Ethernet'), isNull);
      expect(WifiNames.netshName(_netshNoLocation, 'Wi-Fi'), isNull);
    });
  });

  group('Each system', () {
    test('Linux asks iw about that interface, then nmcli', () async {
      final asked = <List<String>>[];
      final names = WifiNames(
        os: 'linux',
        run: (executable, arguments) async {
          asked.add([executable, ...arguments]);
          return executable == 'nmcli' ? _nmcli : null;
        },
      );
      expect(await names.of('wlan1'), isA<WifiNamed>().having((named) => named.name, 'name', 'PL-AF103-1A2B'));
      expect(asked.first, ['iw', 'dev', 'wlan1', 'link']);
      expect(asked.last, containsAllInOrder(['nmcli', 'ifname', 'wlan1']));
    });

    test('a helper that hangs is killed and names nothing', () async {
      final watch = Stopwatch()..start();
      final (String shell, List<String> sleep) = Platform.isWindows
          ? ('powershell', ['-NoProfile', '-Command', 'Start-Sleep 30'])
          : ('sh', ['-c', 'sleep 30']);
      expect(await WifiNames.runHelper(shell, sleep), isNull);
      expect(watch.elapsed, lessThan(WifiNames.helperWait * 2));
    });

    test('a helper that answers is read', () async {
      final (String shell, List<String> echo) = Platform.isWindows
          ? ('cmd', ['/c', 'echo SSID'])
          : ('sh', ['-c', 'echo SSID']);
      expect((await WifiNames.runHelper(shell, echo))?.trim(), 'SSID');
    });

    test('Linux with neither tool names nothing', () async {
      final names = WifiNames(os: 'linux', run: (_, _) async => null);
      expect(await names.of('wlan0'), isA<WifiUnnamed>());
    });

    test('Windows reads netsh', () async {
      final names = WifiNames(os: 'windows', run: (_, _) async => _netsh);
      expect(await names.of('Wi-Fi 2'), isA<WifiNamed>().having((named) => named.name, 'name', 'PL-AF103-1A2B'));
    });

    testWidgets('a Mac asks the runner, and an answer with no name is none', (tester) async {
      const channel = MethodChannel('test/desktop-wifi');
      Object? ssid = 'PL-AF103-1A2B';
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (call) async {
        expect(call.method, 'ssid');
        expect(call.arguments, 'en0');
        return {'ssid': ssid};
      });
      final names = WifiNames(os: 'macos', channel: channel);
      expect(await names.of('en0'), isA<WifiNamed>());
      ssid = null;
      expect(await names.of('en0'), isA<WifiUnnamed>());
    });

    testWidgets('a Mac whose location prompt goes unanswered names nothing after the wait', (tester) async {
      const channel = MethodChannel('test/silent-wifi');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (_) => Completer<Object?>().future);
      WifiName? named;
      unawaited(WifiNames(os: 'macos', channel: channel).of('en0').then((given) => named = given));
      await tester.pump(WifiNames.macWait);
      expect(named, isA<WifiUnnamed>());
    });

    testWidgets('a Mac name with a control character is none', (tester) async {
      const channel = MethodChannel('test/odd-wifi');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (_) async => {'ssid': 'PL-AF103-\nab'});
      expect(await WifiNames(os: 'macos', channel: channel).of('en0'), isA<WifiUnnamed>());
    });

    testWidgets('a Mac whose runner fails or has no channel names nothing', (tester) async {
      const channel = MethodChannel('test/failing-wifi');
      Exception failure = PlatformException(code: 'denied');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (_) async => throw failure);
      final names = WifiNames(os: 'macos', channel: channel);
      expect(await names.of('en0'), isA<WifiUnnamed>());
      failure = MissingPluginException();
      expect(await names.of('en0'), isA<WifiUnnamed>());
    });
  });
}
