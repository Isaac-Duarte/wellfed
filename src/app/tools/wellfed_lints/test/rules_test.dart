import 'package:analyzer/analysis_rule/analysis_rule.dart';
import 'package:analyzer_testing/analysis_rule/analysis_rule.dart';
import 'package:test_reflective_loader/test_reflective_loader.dart';
import 'package:wellfed_lints/src/rules.dart';

void main() {
  defineReflectiveSuite(() {
    defineReflectiveTests(StateImportsNoUiTest);
    defineReflectiveTests(UiNoPlatformTest);
    defineReflectiveTests(UiNoBridgeCallsTest);
    defineReflectiveTests(NoWidgetHelperTest);
    defineReflectiveTests(FileLengthTest);
    defineReflectiveTests(TimersTest);
    defineReflectiveTests(TimersInStateTest);
    defineReflectiveTests(CatchTest);
    defineReflectiveTests(CatchInStateTest);
    defineReflectiveTests(CatchInTestsTest);
    defineReflectiveTests(NoBangTest);
    defineReflectiveTests(SingleStateTest);
  });
}

/// A rule's test over a file in one layer, beside a package called `wellfed`.
abstract class _LayerTest extends AnalysisRuleTest {
  AnalysisRule get subject;

  String get layer;

  @override
  String get testFileName => '$layer/subject.dart';

  @override
  void setUp() {
    rule = subject;
    newPackage('wellfed')
      ..addFile('lib/screens/a.dart', 'class A {}')
      ..addFile('lib/widgets/b.dart', 'class B {}')
      ..addFile('lib/platform/c.dart', 'class C {}')
      ..addFile('lib/state/d.dart', 'class D {}')
      ..addFile(
        'lib/src/rust/api/e.dart',
        'class Session { void feed() {} }\nvoid hello() {}\n'
            'class Browse { factory Browse() => Browse._(); Browse._(); }\n'
            'class Rate { const Rate(); }\n'
            'class Fed { const factory Fed() = _Fed; }\nclass _Fed implements Fed { const _Fed(); }\n',
      );
    super.setUp();
  }
}

@reflectiveTest
class StateImportsNoUiTest extends _LayerTest {
  @override
  AnalysisRule get subject => StateImportsNoUi();

  @override
  String get layer => 'state';

  Future<void> test_screens() async {
    await assertDiagnostics("// ignore: unused_import\nimport 'package:wellfed/screens/a.dart';\n", [lint(25, 40)]);
  }

  Future<void> test_state() async {
    await assertNoDiagnostics("// ignore: unused_import\nimport 'package:wellfed/state/d.dart';\n");
  }
}

@reflectiveTest
class UiNoPlatformTest extends _LayerTest {
  @override
  AnalysisRule get subject => UiNoPlatform();

  @override
  String get layer => 'widgets';

  Future<void> test_platform() async {
    await assertDiagnostics("// ignore: unused_import\nimport 'package:wellfed/platform/c.dart';\n", [lint(25, 41)]);
  }

  Future<void> test_state() async {
    await assertNoDiagnostics("// ignore: unused_import\nimport 'package:wellfed/state/d.dart';\n");
  }
}

@reflectiveTest
class UiNoBridgeCallsTest extends _LayerTest {
  @override
  AnalysisRule get subject => UiNoBridgeCalls();

  @override
  String get layer => 'screens';

  Future<void> test_function() async {
    await assertDiagnostics("import 'package:wellfed/src/rust/api/e.dart';\nvoid f() => hello();\n", [lint(58, 5)]);
  }

  Future<void> test_method() async {
    await assertDiagnostics("import 'package:wellfed/src/rust/api/e.dart';\nvoid f(Session s) => s.feed();\n", [
      lint(69, 4),
    ]);
  }

  Future<void> test_naming_a_type() async {
    await assertNoDiagnostics("import 'package:wellfed/src/rust/api/e.dart';\nSession? f(Session? s) => s;\n");
  }

  Future<void> test_a_factory_that_runs_in_rust() async {
    const code = "import 'package:wellfed/src/rust/api/e.dart';\nObject f() => Browse();\n";
    await assertDiagnostics(code, [lint(code.indexOf('Browse()'), 'Browse'.length)]);
  }

  Future<void> test_a_value_built_in_dart() async {
    await assertNoDiagnostics(
      "import 'package:wellfed/src/rust/api/e.dart';\nList<Object> f() => [const Rate(), const Fed()];\n",
    );
  }
}

@reflectiveTest
class NoWidgetHelperTest extends _LayerTest {
  @override
  AnalysisRule get subject => NoWidgetHelper();

  @override
  String get layer => 'screens';

  static const _widget = 'class Widget {}\nclass Text extends Widget {}\n';

  Future<void> test_method() async {
    await assertDiagnostics('${_widget}class S { Text _t() => Text(); Text u() => _t(); }\n', [
      lint(60, 2),
      lint(81, 1),
    ]);
  }

  Future<void> test_function() async {
    await assertDiagnostics('${_widget}Widget t() => Text();\n', [lint(52, 1)]);
  }

  Future<void> test_override_and_closure() async {
    await assertNoDiagnostics(
      '${_widget}abstract class B { Object build(); }\n'
      'class S extends B { @override Widget build() => Text(); }\n'
      'final Widget Function() make = () => Text();\n',
    );
  }
}

@reflectiveTest
class FileLengthTest extends _LayerTest {
  @override
  AnalysisRule get subject => FileLength();

  @override
  String get layer => 'screens';

  Future<void> test_over() async {
    await assertDiagnostics('${'// line\n' * maxLines}void f() {}\n', [lint(maxLines * 8, 4)]);
  }

  Future<void> test_over_in_trailing_comments() async {
    await assertDiagnostics('void f() {}\n${'// line\n' * maxLines}', [lint(0, 4)]);
  }

  Future<void> test_at_the_limit() async {
    await assertNoDiagnostics('${'// line\n' * (maxLines - 1)}void f() {}\n');
  }
}

@reflectiveTest
class TimersTest extends _LayerTest {
  @override
  AnalysisRule get subject => TimersInStateOrPlatform();

  @override
  String get layer => 'widgets';

  Future<void> test_timer() async {
    await assertDiagnostics("import 'dart:async';\nconst d = Duration(seconds: 1);\nfinal t = Timer(d, () {});\n", [
      lint(63, 15),
    ]);
  }

  Future<void> test_delayed() async {
    await assertDiagnostics('const d = Duration(seconds: 1);\nfinal t = Future<void>.delayed(d);\n', [lint(42, 23)]);
  }
}

@reflectiveTest
class TimersInStateTest extends _LayerTest {
  @override
  AnalysisRule get subject => TimersInStateOrPlatform();

  @override
  String get layer => 'state';

  Future<void> test_named_duration() async {
    await assertNoDiagnostics("import 'dart:async';\nconst d = Duration(seconds: 1);\nfinal t = Timer(d, () {});\n");
  }

  Future<void> test_a_protocol_value() async {
    await assertNoDiagnostics("import 'dart:async';\nfinal d = 3;\nfinal t = Timer(Duration(seconds: d), () {});\n");
  }

  Future<void> test_literal_duration() async {
    await assertDiagnostics("import 'dart:async';\nfinal t = Timer(Duration(seconds: 1), () {});\n", [lint(37, 20)]);
  }
}

@reflectiveTest
class CatchTest extends _LayerTest {
  @override
  AnalysisRule get subject => CatchInControllers();

  @override
  String get layer => 'screens';

  Future<void> test_catch() async {
    await assertDiagnostics('void f() { try { f(); } on Exception { f(); } }\n', [lint(24, 2)]);
  }

  Future<void> test_catch_error() async {
    const code = 'void f(Future<void> x) { x.catchError((Object _) {}); }\n';
    await assertDiagnostics(code, [lint(code.indexOf('catchError'), 'catchError'.length)]);
  }
}

/// A checkout that sits under some other directory named `lib`.
@reflectiveTest
class CatchInTestsTest extends AnalysisRuleTest {
  @override
  String get testPackageRootPath => '$workspaceRootPath/lib/checkout';

  @override
  String get testFilePath => '$testPackageRootPath/test/subject_test.dart';

  @override
  void setUp() {
    rule = CatchInControllers();
    super.setUp();
  }

  Future<void> test_catch() async {
    await assertNoDiagnostics('void f() { try { f(); } on Exception { f(); } }\n');
  }
}

@reflectiveTest
class CatchInStateTest extends _LayerTest {
  @override
  AnalysisRule get subject => CatchInControllers();

  @override
  String get layer => 'state';

  Future<void> test_catch() async {
    await assertNoDiagnostics('void f() { try { f(); } on Exception { f(); } }\n');
  }
}

@reflectiveTest
class NoBangTest extends _LayerTest {
  @override
  AnalysisRule get subject => NoBang();

  @override
  String get layer => 'state';

  Future<void> test_bang() async {
    await assertDiagnostics('int f(int? a) => a!;\n', [lint(18, 1)]);
  }

  Future<void> test_negation() async {
    await assertNoDiagnostics('bool f(bool a) => !a;\n');
  }
}

@reflectiveTest
class SingleStateTest extends _LayerTest {
  @override
  AnalysisRule get subject => SingleState();

  @override
  String get layer => 'state';

  static const _notifier = 'class ChangeNotifier {}\n';

  Future<void> test_public_field() async {
    await assertDiagnostics('${_notifier}class C extends ChangeNotifier { int count = 0; }\n', [lint(61, 5)]);
  }

  Future<void> test_private_and_final() async {
    await assertNoDiagnostics(
      '${_notifier}class C extends ChangeNotifier {\n  int _a = 0;\n  final int b = 0;\n  int get a => _a;\n}\n',
    );
  }

  Future<void> test_not_a_controller() async {
    await assertNoDiagnostics('class C { int count = 0; }\n');
  }
}
