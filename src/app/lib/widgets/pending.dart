import 'package:flutter/material.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/widgets/replies.dart';

/// What a [Pending] control calls to move.
abstract interface class PendingWrite<T> {
  /// Show `value` without asking for it, as while a finger holds a slider.
  void hold(T value);

  /// Show `value` and ask for it through `send`, the write's one trip.
  Future<void> ask(T value, Future<FeederReply> Function() send);
}

/// A control that moves on tap. It shows the value asked for from the tap
/// until the feeder's card agrees, then the card's again. The session puts a
/// write's `Done` on the card as its caller hears it, so one that worked never
/// jumps back. A refusal, or a write that never went, moves the control back
/// and says why. One with no answer says so too and shows what the card says,
/// since it may or may not have run.
///
/// A `Done` the card never takes up (the feeder's writer could not make it,
/// or another change came first) ends once the card's generation reaches the
/// one the `Done` named, or for a store with none, at the card's next change:
/// the control then shows what the feeder holds.
///
/// Writes queue in Rust, one at a time on the latest generation, so the
/// control stays usable while one is out.
class Pending<T> extends StatefulWidget {
  const Pending({super.key, required this.card, this.generation, required this.builder});

  /// What the feeder's card says.
  final T card;

  /// The generation of the store the card's value comes from, if it has one.
  final int? generation;

  /// The control, showing `value`.
  final Widget Function(BuildContext context, T value, PendingWrite<T> write) builder;

  @override
  State<Pending<T>> createState() => _PendingState<T>();
}

/// A write the feeder said it took, while the card does not show it yet.
sealed class _Answered {
  const _Answered();
}

class _NotYet extends _Answered {
  const _NotYet();
}

/// Done, naming the store's generation after it, or with none.
class _Done extends _Answered {
  const _Done(this.generation);

  final int? generation;
}

class _PendingState<T> extends State<Pending<T>> implements PendingWrite<T> {
  /// What was asked for, while the card does not show it yet.
  (T,)? _asked;

  _Answered _answered = const _NotYet();

  /// The newest ask, so an older one's reply cannot settle a newer value.
  int _ticket = 0;

  /// Asks whose reply has not come. While any is out, a card that agrees
  /// does not settle the value: on, then off, tapped quickly would show the
  /// first write's card on the way.
  int _out = 0;

  /// Whether a `Done` naming `done` is settled by a card at `generation`,
  /// or for a store with no generation, by a card that moved.
  bool _settled(int? done, {required bool moved}) => switch ((done, widget.generation)) {
    (final int done, final int generation) => generation >= done,
    _ => moved,
  };

  void _clear() {
    _asked = null;
    _answered = const _NotYet();
  }

  @override
  void didUpdateWidget(Pending<T> old) {
    super.didUpdateWidget(old);
    final (T,)? asked = _asked;
    if (asked == null) return;
    if (widget.card == asked.$1 && _out == 0) {
      _clear();
    } else if (_answered case _Done(:final generation)
        when _settled(generation, moved: widget.card != old.card || widget.generation != old.generation)) {
      _clear();
    }
  }

  @override
  void hold(T value) => setState(() {
    _ticket++;
    _asked = (value,);
    _answered = const _NotYet();
  });

  @override
  Future<void> ask(T value, Future<FeederReply> Function() send) async {
    final int ticket = ++_ticket;
    setState(() {
      _asked = (value,);
      _answered = const _NotYet();
      _out++;
    });

    final FeederReply reply = await _sent(send);
    _out--;
    if (!mounted) return;
    if (ticket == _ticket) {
      setState(() {
        if (reply case FeederReply_Done(:final generation)
            when widget.card != value && !_settled(generation, moved: false)) {
          _answered = _Done(generation);
        } else {
          _clear();
        }
      });
    }
    showProblem(context, reply);
  }

  /// What `send` answered, or an unknown end when it threw (the bridge
  /// failing, a panic among them): the write may have gone, so it is never
  /// sent again, and the count of writes out still comes down.
  static Future<FeederReply> _sent(Future<FeederReply> Function() send) async {
    try {
      return await send();
    } on Object catch (error) {
      debugPrint('a write ended with no answer: $error');
      return const FeederReply.unanswered();
    }
  }

  @override
  Widget build(BuildContext context) => widget.builder(context, switch (_asked) {
    (final T asked,) => asked,
    null => widget.card,
  }, this);
}
