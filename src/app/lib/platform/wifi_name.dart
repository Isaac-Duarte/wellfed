import 'dart:io';

import 'package:flutter/services.dart';

/// The Wi-Fi network one network interface is on, as a computer says. Only a
/// name read from the system counts: anything else is [WifiUnnamed].
sealed class WifiName {
  const WifiName();
}

/// The interface is on the Wi-Fi network [name].
final class WifiNamed extends WifiName {
  /// On `name`.
  const WifiNamed(this.name);

  /// The network's name (its SSID).
  final String name;
}

/// The system did not say which Wi-Fi the interface is on: it is not a Wi-Fi
/// interface, the tool that says is missing, or the system withholds the name
/// from apps not allowed the computer's location (macOS, and Windows 11 since
/// 24H2).
final class WifiUnnamed extends WifiName {
  /// Unnamed.
  const WifiUnnamed();
}

/// Runs a helper and gives its standard output, or `null` when it could not
/// run or failed.
typedef RunHelper = Future<String?> Function(String executable, List<String> arguments);

/// Reads the Wi-Fi name of one interface on each desktop:
///
/// - Linux: NetworkManager's access point in use on that device
///   (`nmcli … device wifi list ifname <if>`), else `iw dev <if> link`.
///   NetworkManager's own "Wi-Fi name" for the computer is the profile holding
///   the default route, which on a cable is the wired one, so it can't say.
/// - Windows: `netsh wlan show interfaces`, the block for that interface.
/// - macOS: CoreWLAN through the runner, which asks for location first.
class WifiNames {
  /// Reads through `run` and `channel`, the system's unless a test gives
  /// others, on the operating system `os`.
  WifiNames({RunHelper? run, MethodChannel? channel, String? os})
    : _run = run ?? _runHelper,
      _channel = channel ?? const MethodChannel('io.github.isaacduarte.wellfed/desktop-wifi'),
      _os = os ?? Platform.operatingSystem;

  final RunHelper _run;
  final MethodChannel _channel;
  final String _os;

  /// The Wi-Fi network `interface` is on.
  Future<WifiName> of(String interface) async {
    final String? name = switch (_os) {
      'linux' => await _linux(interface),
      'windows' => netshName(await _run('netsh', ['wlan', 'show', 'interfaces']), interface),
      'macos' => await _mac(interface),
      _ => null,
    };
    return name == null || name.isEmpty ? const WifiUnnamed() : WifiNamed(name);
  }

  Future<String?> _linux(String interface) async =>
      nmcliName(
        await _run('nmcli', ['-g', 'IN-USE,SSID', 'device', 'wifi', 'list', 'ifname', interface, '--rescan', 'no']),
      ) ??
      iwName(await _run('iw', ['dev', interface, 'link']));

  Future<String?> _mac(String interface) async {
    try {
      final Map<String, Object?>? answer = await _channel.invokeMapMethod<String, Object?>('ssid', interface);
      return switch (answer?['ssid']) {
        final String name => name,
        _ => null,
      };
    } on MissingPluginException {
      return null;
    } on PlatformException {
      return null;
    }
  }

  /// The access point in use in `nmcli -g IN-USE,SSID … wifi list` output:
  /// the line marked `*`. `-g` escapes `:` and `\` in values with `\`.
  static String? nmcliName(String? output) {
    if (output == null) return null;
    for (final String line in output.split('\n')) {
      if (!line.startsWith('*:')) continue;
      return line.substring(2).replaceAllMapped(RegExp(r'\\(.)'), (match) => match.group(1) ?? '');
    }
    return null;
  }

  /// The `SSID:` line of `iw dev <if> link`, which says `Not connected.`
  /// otherwise.
  static String? iwName(String? output) {
    if (output == null) return null;
    for (final String line in output.split('\n')) {
      final String trimmed = line.trim();
      if (trimmed.startsWith('SSID: ')) return trimmed.substring('SSID: '.length);
    }
    return null;
  }

  /// The SSID in `netsh wlan show interfaces` output for the interface
  /// `name`. Each interface is a block of `key : value` lines whose first is
  /// its name, under a label Windows translates; `SSID` it does not.
  static String? netshName(String? output, String name) {
    if (output == null) return null;
    String? firstKey;
    String? block;
    for (final String line in output.split('\n')) {
      // The fields are indented; the heading above them is not.
      final int colon = line.indexOf(':');
      if (colon < 0 || !line.startsWith(' ')) continue;
      final String key = line.substring(0, colon).trim();
      final String value = line.substring(colon + 1).trim();
      if (key.isEmpty || value.isEmpty) continue;
      firstKey ??= key;
      if (key == firstKey) {
        block = value;
      } else if (key == 'SSID' && block == name) {
        return value;
      }
    }
    return null;
  }

  static Future<String?> _runHelper(String executable, List<String> arguments) async {
    try {
      final ProcessResult result = await Process.run(executable, arguments);
      return result.exitCode == 0 ? '${result.stdout}' : null;
    } on ProcessException {
      // Not installed here: the name can't be read this way.
      return null;
    }
  }
}
