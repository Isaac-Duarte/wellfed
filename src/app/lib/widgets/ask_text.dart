import 'package:flutter/material.dart';

/// Ask for one line of text. The dialog owns its controller, because its field
/// still draws through the closing animation after the answer comes back.
Future<String?> askText(
  BuildContext context, {
  required String title,
  required String label,
  String initial = '',
  TextInputType? keyboard,
  String action = 'Save',
  String? note,
}) => showDialog<String>(context: context, builder: (_) => _AskText(title, label, initial, keyboard, action, note));

class _AskText extends StatefulWidget {
  const _AskText(this.title, this.label, this.initial, this.keyboard, this.action, this.note);

  final String title;
  final String label;
  final String initial;
  final TextInputType? keyboard;
  final String action;

  /// A line under the field on what the answer changes.
  final String? note;

  @override
  State<_AskText> createState() => _AskTextState();
}

class _AskTextState extends State<_AskText> {
  late final _controller = TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(widget.title),
    content: TextField(
      controller: _controller,
      autofocus: true,
      keyboardType: widget.keyboard,
      decoration: InputDecoration(labelText: widget.label, helperText: widget.note, helperMaxLines: 3),
    ),
    actions: [
      TextButton(onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
      FilledButton(onPressed: () => Navigator.pop(context, _controller.text), child: Text(widget.action)),
    ],
  );
}
