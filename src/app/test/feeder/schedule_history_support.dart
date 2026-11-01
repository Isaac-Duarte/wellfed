import 'package:wellfed/src/rust/api/feeder.dart';
import 'package:wellfed/src/rust/api/model.dart';

PlanView planPlan(
  int minute, {
  int portions = 2,
  List<Weekday> days = Weekday.values,
  bool enabled = true,
  int call = 0,
}) => PlanView(minute: minute, portions: portions, days: days, enabled: enabled, call: call);

PlansView planPlans(List<PlanView?> filled, {int generation = 3}) =>
    PlansView(generation: generation, writable: true, plans: [...filled, ...List.filled(10 - filled.length, null)]);

const planNoRecording = FeederReply.call(call: CallInfo(writable: true, clip: CallClip.none()));
