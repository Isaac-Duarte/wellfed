import 'dart:async';

import 'package:flutter/material.dart' hide Card;
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:wellfed/screens/feeder/meal_choices.dart';
import 'package:wellfed/screens/feeder/meal_editor.dart';
import 'package:wellfed/screens/feeder/schedule.dart';
import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';
import 'package:wellfed/src/rust/api/session.dart';
import 'package:wellfed/state/feeder/feeder.dart';

import '../support/fakes.dart';
import '../support/harness.dart';
import 'schedule_history_support.dart';

void main() {
  group('Schedule', () {
    testWidgets('each meal is its own node, with its switch its own switch', (tester) async {
      final SemanticsHandle semantics = tester.ensureSemantics();
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(fakeCard(plans: planPlans([planPlan(60), planPlan(120)])));
      await showTab(tester, ScheduleTab(feeder: feeder));

      // Offline is the hard case: with no tap on the row or the switch, nothing
      // else keeps them from merging into one node.
      unawaited(session.updates.close());
      await tester.pump();
      await tester.pump(const Duration(seconds: 3));
      await settle(tester);

      final Finder switches = find.byType(Switch);
      expect(switches, findsNWidgets(2));
      final SemanticsNode first = tester.getSemantics(switches.first);
      expect(first.label, '1:00 AM on');
      expect(first.rect.width, lessThanOrEqualTo(tester.getSize(switches.first).width));

      final SemanticsNode row = tester.getSemantics(find.text('1:00 AM'));
      expect(row.label, isNot(contains('2:00 AM')));

      semantics.dispose();
    });

    testWidgets('coming to the tab reads the plans again, and only when online', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(fakeCard());
      await settle(tester);
      session.requests.clear();

      await showTab(tester, ScheduleTab(feeder: feeder));

      expect(session.requests.single, const FeederRequest.getPlans());
    });

    Future<FakeSession> show(WidgetTester tester, PlansView plans, {bool clockSet = true}) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(fakeCard(plans: plans, clockSet: clockSet));
      await showTab(tester, ScheduleTab(feeder: feeder));
      session.requests.clear();
      return session;
    }

    testWidgets('marks the meal the feeder runs next', (tester) async {
      final (FeederController feeder, FakeSession session) = await oneFeeder(tester);
      session.status(
        fakeCard(
          plans: planPlans([planPlan(7 * 60 + 30), planPlan(18 * 60)]),
          nextMeal: NextMealView(
            slot: 1,
            atMs: DateTime.now().add(const Duration(hours: 1)).millisecondsSinceEpoch,
            minute: 18 * 60,
            portions: 2,
          ),
        ),
      );
      await showTab(tester, ScheduleTab(feeder: feeder));

      expect(find.text('Next'), findsOneWidget);
      final double chip = tester.getCenter(find.text('Next')).dy;
      expect(chip, closeTo(tester.getCenter(find.text('6:00 PM')).dy, 2));
    });

    testWidgets('lists the meals by time, with their days and calls', (tester) async {
      await show(
        tester,
        planPlans([
          planPlan(18 * 60 + 30, portions: 3, call: 2),
          planPlan(7 * 60 + 30),
          planPlan(12 * 60 + 30, days: const [Weekday.mon, Weekday.tue, Weekday.wed, Weekday.thu, Weekday.fri]),
          planPlan(22 * 60, portions: 1, days: const [], enabled: false),
        ]),
      );

      expect(find.text('4 of 10 meals'), findsOneWidget);
      expect(find.text('2 portions · every day'), findsOneWidget);
      expect(find.text('2 portions · Mon to Fri'), findsOneWidget);
      expect(find.text('3 portions · every day · meal call twice'), findsOneWidget);
      expect(find.text('Off · 1 portion · once'), findsOneWidget);
      final double first = tester.getTopLeft(find.text('7:30 AM')).dy;
      final double last = tester.getTopLeft(find.text('10:00 PM')).dy;
      expect(first, lessThan(last));
    });

    testWidgets('a feeder at its limit offers no new meal', (tester) async {
      await show(tester, planPlans([for (var i = 0; i < 10; i++) planPlan(i * 60)]));

      expect(find.textContaining('10 of 10 meals'), findsOneWidget);
      final Finder add = find.widgetWithText(FilledButton, 'Add meal · delete one first');
      expect(tester.widget<FilledButton>(add).onPressed, isNull);
    });

    testWidgets('meals on hold offer the time', (tester) async {
      await show(tester, planPlans([planPlan(60)]), clockSet: false);

      expect(find.text('Set the time from this phone'), findsOneWidget);
    });

    testWidgets('a meal is switched once, against what the feeder said, and moves on tap', (tester) async {
      final PlansView plans = planPlans([planPlan(60)]);
      final FakeSession session = await show(tester, plans);
      final answer = Completer<FeederReply>();
      session.answering = (_) => answer.future;

      await tapIn(tester, find.byType(Switch));
      expect(session.requests, [
        FeederRequest.editPlans(plans: plans, edit: const PlanEdit.enabled(slot: 0, enabled: false)),
      ]);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
      answer.complete(const FeederReply.done(generation: 4));
      await settle(tester);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isFalse);
    });

    testWidgets('switching says why it was refused or may have run, and moves back', (tester) async {
      final FakeSession session = await show(tester, planPlans([planPlan(60)]));
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.busy()))
        ..add(const FeederReply.unanswered());

      await tapIn(tester, find.byType(Switch));
      expect(find.textContaining("It's busy"), findsOneWidget);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      await tapIn(tester, find.byType(Switch));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(tester.widget<Switch>(find.byType(Switch)).value, isTrue);
      expect(session.requests, hasLength(2));
    });

    testWidgets('a new meal goes in as an add', (tester) async {
      final PlansView plans = planPlans([planPlan(60)]);
      final FakeSession session = await show(tester, plans);

      await tapIn(tester, find.text('Add meal'));
      await tapIn(tester, find.byTooltip('One more'));
      await tapIn(tester, find.text('Weekends'));
      // A meal not saved yet has no switch to turn it off.
      expect(find.descendant(of: find.byType(MealEditor), matching: find.byType(Switch)), findsNothing);
      await tapIn(tester, find.widgetWithText(Segment, '2'));
      await tapIn(tester, find.text('Save'));

      final added = session.requests.single as FeederRequest_EditPlans;
      expect(added.plans, plans);
      final PlanView plan = (added.edit as PlanEdit_Add).plan;
      expect(plan.minute, 8 * 60);
      expect(plan.portions, 2);
      expect(plan.days, [Weekday.sat, Weekday.sun]);
      expect(plan.call, 2);
      expect(plan.enabled, isTrue);
      expect(find.text('New meal'), findsNothing, reason: 'it closes');
    });

    testWidgets('an edit changes its slot, or deletes it', (tester) async {
      final PlansView plans = planPlans([planPlan(60, portions: 3)]);
      final FakeSession session = await show(tester, plans);

      await tapIn(tester, find.text('1:00 AM'));
      await tapIn(tester, find.byTooltip('One fewer'));
      await tapIn(tester, find.text('Save'));
      final change = session.requests.single as FeederRequest_EditPlans;
      final changed = change.edit as PlanEdit_Change;
      expect(changed.slot, 0);
      expect(changed.minute, 60);
      expect(changed.portions, 2);
      expect(changed.days, Weekday.values);
      expect(changed.enabled, isTrue);

      session.requests.clear();
      await tapIn(tester, find.text('1:00 AM'));
      await tapIn(tester, find.text('Delete meal'));
      await tapIn(tester, find.widgetWithText(FilledButton, 'Delete'));
      expect((session.requests.single as FeederRequest_EditPlans).edit, const PlanEdit.delete(slot: 0));
    });

    testWidgets("the editor's switch is its own node, named On", (tester) async {
      final SemanticsHandle semantics = tester.ensureSemantics();
      await show(tester, planPlans([planPlan(60)]));

      await tapIn(tester, find.text('1:00 AM'));
      final Finder toggle = find.descendant(of: find.byType(MealEditor), matching: find.byType(Switch));
      await tester.ensureVisible(toggle);
      await tester.pump();

      final SemanticsNode node = tester.getSemantics(toggle);
      expect(node.label, 'On');
      expect(node.rect.width, lessThanOrEqualTo(tester.getSize(toggle).width));

      semantics.dispose();
    });

    testWidgets('deleting a meal asks first, naming its time', (tester) async {
      final FakeSession session = await show(tester, planPlans([planPlan(60)]));
      session.requests.clear();

      await tapIn(tester, find.text('1:00 AM'));
      await tapIn(tester, find.text('Delete meal'));

      expect(find.text('Delete this meal?'), findsOneWidget);
      expect(find.textContaining('1:00 AM meal'), findsOneWidget);
      expect(session.requests, isEmpty);

      await tapIn(tester, find.text('Cancel'));
      expect(find.text('Delete this meal?'), findsNothing);
      expect(find.text('Delete meal'), findsOneWidget);
      expect(session.requests, isEmpty);
    });

    testWidgets('a refused or unanswered save keeps the editor open', (tester) async {
      final FakeSession session = await show(tester, planPlans([planPlan(60)]));
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.forbidden()))
        ..add(const FeederReply.unanswered());

      await tapIn(tester, find.text('1:00 AM'));
      await tapIn(tester, find.text('Save'));
      expect(find.textContaining("isn't allowed"), findsOneWidget);
      expect(find.text('Delete meal'), findsOneWidget);

      await tapIn(tester, find.text('Save'));
      expect(find.textContaining('may or may not have'), findsOneWidget);
      expect(find.text('Delete meal'), findsOneWidget);
      expect(session.requests, hasLength(2));
    });

    testWidgets("someone else's change is shown, and only overwritten on a "
        'choice, against what the feeder now holds', (tester) async {
      final FakeSession session = await show(tester, planPlans([planPlan(60)]));
      final PlansView theirs = planPlans([planPlan(120)], generation: 4);

      await tapIn(tester, find.text('1:00 AM'));
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.stale(current: 4)))
        ..add(FeederReply.plans(plans: theirs));
      await tapIn(tester, find.text('Save'));

      expect(find.text('The meals changed while you were editing'), findsOneWidget);
      expect(find.text('2:00 AM · 2 portions'), findsOneWidget);
      expect(find.text('1:00 AM · 2 portions'), findsOneWidget);
      expect(session.requests, hasLength(2), reason: 'the save, then a read');

      await tapIn(tester, find.text('Save mine over it'));
      final again = session.requests.last as FeederRequest_EditPlans;
      expect(again.plans, theirs);
      expect(session.requests, hasLength(3));
    });

    testWidgets('a delete refused as stale shows the meal now, sends nothing more, '
        'and goes only on a choice, against the fresh read', (tester) async {
      final FakeSession session = await show(tester, planPlans([planPlan(60)]));
      final PlansView fresh = planPlans([planPlan(60)], generation: 4);

      await tapIn(tester, find.text('1:00 AM'));
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.stale(current: 4)))
        ..add(FeederReply.plans(plans: fresh));
      await tapIn(tester, find.text('Delete meal'));
      await tapIn(tester, find.widgetWithText(FilledButton, 'Delete'));

      expect(find.text('The meals changed while you were editing'), findsOneWidget);
      expect(find.textContaining('a one-off meal ran'), findsOneWidget);
      expect(find.text('1:00 AM · 2 portions'), findsOneWidget);
      expect(session.requests, hasLength(2), reason: 'the delete, then a read');
      expect(session.requests.last, const FeederRequest.getPlans());

      await tapIn(tester, find.text('Delete it'));
      final again = session.requests.last as FeederRequest_EditPlans;
      expect(again.plans, fresh);
      expect(session.requests, hasLength(3));
      expect(find.text('Delete meal'), findsNothing);
    });

    testWidgets('a stale switch reads the plans and does not switch again', (tester) async {
      final FakeSession session = await show(tester, planPlans([planPlan(60)]));
      session.replies.add(const FeederReply.refused(refusal: FeederRefusal.stale(current: 4)));

      await tapIn(tester, find.byType(Switch).first);

      expect(session.requests, hasLength(2));
      expect(session.requests.first, isA<FeederRequest_EditPlans>());
      expect(session.requests.last, const FeederRequest.getPlans());
      expect(find.textContaining('This changed just now'), findsOneWidget);
    });

    testWidgets('keeping theirs sends nothing more', (tester) async {
      final FakeSession session = await show(tester, planPlans([planPlan(60)]));

      await tapIn(tester, find.text('1:00 AM'));
      session.replies
        ..add(const FeederReply.refused(refusal: FeederRefusal.stale(current: 4)))
        ..add(FeederReply.plans(plans: planPlans([planPlan(120)])));
      await tapIn(tester, find.text('Save'));
      await tapIn(tester, find.text("Keep what's there"));

      expect(session.requests, hasLength(2));
      expect(find.text('Delete meal'), findsNothing);
    });
  });
}
