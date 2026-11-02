import 'package:flutter/material.dart';

/// Public time servers an owner can pick instead of typing one, for a router
/// that answers no NTP. They are addresses, not names, because the feeder
/// resolves no names for NTP: it asks exactly the address it was given.
const List<({String name, String ip})> publicNtpServers = [
  (name: 'Cloudflare', ip: '162.159.200.1'),
  (name: 'Google', ip: '216.239.35.0'),
];

/// Ask which NTP server to use: a tap on one of [publicNtpServers], or any
/// IPv4 address typed into the field below them. The feeder checks the answer.
Future<String?> askNtpServer(BuildContext context) =>
    showDialog<String>(context: context, builder: (_) => const _AskNtp());

class _AskNtp extends StatefulWidget {
  const _AskNtp();

  @override
  State<_AskNtp> createState() => _AskNtpState();
}

class _AskNtpState extends State<_AskNtp> {
  // The dialog owns its controller, because its field still draws through the
  // closing animation after the answer comes back.
  final _controller = TextEditingController();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: const Text('Ask which address?'),
    content: Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final server in publicNtpServers)
          ListTile(
            contentPadding: EdgeInsets.zero,
            title: Text(server.name),
            subtitle: Text(server.ip),
            onTap: () => Navigator.pop(context, server.ip),
          ),
        TextField(
          controller: _controller,
          keyboardType: TextInputType.number,
          decoration: const InputDecoration(labelText: 'Or another IPv4 address'),
        ),
      ],
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(onPressed: () => Navigator.pop(context, _controller.text.trim()), child: const Text('Save')),
    ],
  );
}
