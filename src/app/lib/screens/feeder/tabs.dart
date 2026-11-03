import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:go_router/go_router.dart';
import 'package:wellfed/widgets/desktop.dart';

/// The feeder screen's tabs, for a link to one of them.
const todayTab = 0;
const scheduleTab = 1;
const historyTab = 2;
const settingsTab = 3;

/// Open a feeder at `tab`: over Home on a phone, so Back returns to it, and
/// in place on a desktop, where the sidebar is the way back.
void openFeeder(BuildContext context, String identity, int tab) {
  final location = '/feeder/$identity?tab=$tab';
  if (isDesktop) {
    context.go(location);
  } else {
    unawaited(context.push(location));
  }
}
