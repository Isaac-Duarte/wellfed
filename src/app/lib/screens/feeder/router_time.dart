import 'package:flutter/material.dart';
import 'package:wellfed/screens/feeder/ask_ntp.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/state/feeder/feeder.dart';
import 'package:wellfed/theme/theme.dart';
import 'package:wellfed/widgets/layout.dart';
import 'package:wellfed/widgets/replies.dart';

/// Whether to offer a public time server: the feeder asks its router, which
/// did not answer, and this phone may change where the time comes from. A
/// router that answers loses a packet now and then, and the feeder asks
/// again within seconds, so a clock the router last set is never offered.
///
/// The firmware's default stays the router, so a feeder talks to nothing
/// outside the owner's network unless the owner chooses it here.
bool routerGivesNoTime(TimeInfo? time) =>
    time != null &&
    time.writable &&
    time.ntp is NtpView_Gateway &&
    time.last != TimeSource.ntp &&
    (time.ntpLast == NtpResult.lost || time.ntpLast == NtpResult.refused);

/// The router gives no time: offer Cloudflare's server, or phones alone.
/// Either answer changes the setting, so the offer goes away for good.
class RouterTimeNotice extends StatefulWidget {
  const RouterTimeNotice({super.key, required this.feeder, required this.time});

  final FeederController feeder;
  final TimeInfo time;

  @override
  State<RouterTimeNotice> createState() => _RouterTimeNoticeState();
}

class _RouterTimeNoticeState extends State<RouterTimeNotice> {
  bool _asking = false;

  Future<void> _use(String ntp) async {
    setState(() => _asking = true);
    final FeederReply reply = await widget.feeder.ask(FeederRequest.putTime(base: widget.time.generation, ntp: ntp));
    if (!mounted) return;
    showProblem(context, reply);

    // The feeder writes no event for a time setting, so read it again: the
    // offer goes once the card shows the new source. The buttons wait for
    // it, as a second tap would go against the old generation.
    await widget.feeder.ask(const FeederRequest.getTime());
    if (mounted) setState(() => _asking = false);
  }

  @override
  Widget build(BuildContext context) {
    final ({String name, String ip}) cloudflare = publicNtpServers.first;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const Notice(
          tone: Tone.neutral,
          lead: "Your router doesn't give the time.",
          text:
              'The feeder keeps time only from phones, and meals hold if it '
              "drifts too far between visits. Cloudflare's public time server "
              'can keep it set: the feeder asks it every 17 minutes.',
        ),
        const SizedBox(height: Space.s2),
        FilledButton(onPressed: _asking ? null : () => _use(cloudflare.ip), child: Text('Use ${cloudflare.name}')),
        TextButton(onPressed: _asking ? null : () => _use('off'), child: const Text('Keep phones only')),
      ],
    );
  }
}
