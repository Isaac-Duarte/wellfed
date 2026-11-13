import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:wellfed/widgets/layout.dart';

/// The ports the feeder reaches an installing computer on: stock's broker
/// and the image server, both on TCP.
const _ports = 'TCP ports 1883 and 8088';

/// How this computer's firewall may have kept the feeder out, in its own
/// system's words; `null` on a phone, which serves the feeder unasked.
String? firewallWords() => switch (defaultTargetPlatform) {
  TargetPlatform.macOS =>
    'The feeder connects to this computer on $_ports. If macOS asked whether '
        'Wellfed may accept incoming connections and it was denied, allow it '
        'in System Settings › Network › Firewall › Options.',
  TargetPlatform.windows =>
    'The feeder connects to this computer on $_ports. Windows Defender Firewall '
        'asks the first time Wellfed listens. If that was cancelled, or this '
        'network is set as Public, allow Wellfed for private networks in Windows '
        'Security › Firewall & network protection › Allow an app through firewall.',
  TargetPlatform.linux =>
    'The feeder connects to this computer on $_ports. If firewalld or ufw runs '
        'here, open those ports to your home network: for ufw, '
        'sudo ufw allow 1883,8088/tcp, and for firewalld, '
        'sudo firewall-cmd --add-port=1883/tcp --add-port=8088/tcp.',
  TargetPlatform.android || TargetPlatform.iOS || TargetPlatform.fuchsia => null,
};

/// A computer's firewall may be what kept the feeder out. Nothing on a phone.
class FirewallNotice extends StatelessWidget {
  const FirewallNotice({super.key});

  @override
  Widget build(BuildContext context) {
    final String? words = firewallWords();
    if (words == null) return const SizedBox.shrink();
    return Notice(tone: Tone.neutral, lead: "This computer's firewall may be in the way.", text: words);
  }
}
