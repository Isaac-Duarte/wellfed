import 'package:analyzer/analysis_rule/analysis_rule.dart';
import 'package:analyzer/analysis_rule/rule_context.dart';
import 'package:analyzer/analysis_rule/rule_visitor_registry.dart';
import 'package:analyzer/dart/ast/ast.dart';
import 'package:analyzer/dart/ast/token.dart';
import 'package:analyzer/dart/ast/visitor.dart';
import 'package:analyzer/dart/element/element.dart';
import 'package:analyzer/dart/element/type.dart';
import 'package:analyzer/error/error.dart';

/// Where a file sits in the app, which decides what it may do.
enum Layer { state, screens, widgets, platform, theme, generated, root, outsideLib }

const _bridgeLibrary = 'package:wellfed/src/rust/';

/// The longest a hand-written file may be.
const maxLines = 500;

/// The layer comes from the file's path inside its package, so a checkout
/// that sits under some other directory named `lib` files its tests as
/// tests, not as app code.
Layer layerOf(RuleContext context) {
  final file = context.definingUnit.file;
  final root = context.package?.root;
  if (root == null || !root.contains(file.path)) return Layer.outsideLib;
  final inPackage = file.provider.pathContext.relative(file.path, from: root.path).replaceAll(r'\', '/');
  if (!inPackage.startsWith('lib/')) return Layer.outsideLib;
  final inLib = inPackage.substring('lib/'.length);
  if (inLib.startsWith('src/rust/') || inLib.endsWith('.g.dart')) return Layer.generated;
  if (inLib.startsWith('state/')) return Layer.state;
  if (inLib.startsWith('screens/')) return Layer.screens;
  if (inLib.startsWith('widgets/')) return Layer.widgets;
  if (inLib.startsWith('platform/')) return Layer.platform;
  if (inLib.startsWith('theme/')) return Layer.theme;
  return Layer.root;
}

bool _isUi(Layer layer) => layer == Layer.screens || layer == Layer.widgets;

bool _extends(InterfaceElement? element, String name) =>
    element != null && (element.name == name || element.allSupertypes.any((type) => type.element.name == name));

bool _isWidget(DartType? type) => type is InterfaceType && _extends(type.element, 'Widget');

bool _isOverride(AnnotatedNode node) => node.metadata.any((note) => note.name.name == 'override');

/// A rule over one or more kinds of node in the app's files.
abstract class _Rule extends AnalysisRule {
  _Rule(this.code) : super(name: code.lowerCaseName, description: code.problemMessage);

  final LintCode code;

  @override
  LintCode get diagnosticCode => code;

  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor);

  SimpleAstVisitor<void> visitor(RuleContext context);

  @override
  void registerNodeProcessors(RuleVisitorRegistry registry, RuleContext context) {
    listen(registry, visitor(context));
  }
}

class StateImportsNoUi extends _Rule {
  StateImportsNoUi() : super(_code);

  static const _code = LintCode(
    'state_imports_no_ui',
    'state/ must not import screens/ or widgets/.',
    correctionMessage: 'Move what both need into state/, or pass it in.',
  );

  @override
  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor) =>
      registry.addImportDirective(this, visitor);

  @override
  SimpleAstVisitor<void> visitor(RuleContext context) =>
      _ImportVisitor(this, context, Layer.state, const ['package:wellfed/screens/', 'package:wellfed/widgets/']);
}

class UiNoPlatform extends _Rule {
  UiNoPlatform() : super(_code);

  static const _code = LintCode(
    'ui_no_platform',
    'screens/ and widgets/ must not import platform/.',
    correctionMessage: 'Reach the platform through a controller.',
  );

  @override
  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor) =>
      registry.addImportDirective(this, visitor);

  @override
  SimpleAstVisitor<void> visitor(RuleContext context) =>
      _ImportVisitor(this, context, null, const ['package:wellfed/platform/']);
}

class _ImportVisitor extends SimpleAstVisitor<void> {
  _ImportVisitor(this.rule, this.context, this.layer, this.refused);

  final AnalysisRule rule;
  final RuleContext context;

  /// The layer this applies to; the UI layers when null.
  final Layer? layer;
  final List<String> refused;

  @override
  void visitImportDirective(ImportDirective node) {
    final here = layerOf(context);
    if (layer == null ? !_isUi(here) : here != layer) return;
    final uri = node.uri.stringValue ?? '';
    if (refused.any(uri.startsWith)) rule.reportAtNode(node);
  }
}

class UiNoBridgeCalls extends _Rule {
  UiNoBridgeCalls() : super(_code);

  static const _code = LintCode(
    'ui_no_bridge_calls',
    'screens/ and widgets/ may name generated types but must not call them.',
    correctionMessage: 'Add the call to a controller.',
  );

  @override
  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor) {
    registry.addMethodInvocation(this, visitor);
    registry.addInstanceCreationExpression(this, visitor);
  }

  @override
  SimpleAstVisitor<void> visitor(RuleContext context) => _BridgeVisitor(this, context);
}

class _BridgeVisitor extends SimpleAstVisitor<void> {
  _BridgeVisitor(this.rule, this.context);

  final AnalysisRule rule;
  final RuleContext context;

  @override
  void visitMethodInvocation(MethodInvocation node) {
    if (!_isUi(layerOf(context))) return;
    final element = node.methodName.element;
    if (element == null) return;
    final uri = element.library?.uri.toString() ?? '';
    if (!uri.startsWith(_bridgeLibrary)) return;

    // Generated by freezed: a copy made in Dart, which never reaches Rust.
    if (uri.endsWith('.freezed.dart') || node.methodName.name == 'copyWith') return;
    rule.reportAtNode(node.methodName);
  }

  /// A generated factory that is not a redirect builds its object in Rust; a
  /// plain or freezed constructor only builds a value in Dart.
  @override
  void visitInstanceCreationExpression(InstanceCreationExpression node) {
    if (!_isUi(layerOf(context))) return;
    final constructor = node.constructorName.element;
    if (constructor == null || !constructor.isFactory || constructor.redirectedConstructor != null) return;
    final uri = constructor.library.uri.toString();
    if (uri.startsWith(_bridgeLibrary)) rule.reportAtNode(node.constructorName);
  }
}

class NoWidgetHelper extends _Rule {
  NoWidgetHelper() : super(_code);

  static const _code = LintCode(
    'no_widget_helper',
    'A method or function must not return a Widget.',
    correctionMessage: 'Write a widget class. A builder callback may be a closure.',
  );

  @override
  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor) {
    registry.addMethodDeclaration(this, visitor);
    registry.addFunctionDeclaration(this, visitor);
  }

  @override
  SimpleAstVisitor<void> visitor(RuleContext context) => _HelperVisitor(this, context);
}

class _HelperVisitor extends SimpleAstVisitor<void> {
  _HelperVisitor(this.rule, this.context);

  final AnalysisRule rule;
  final RuleContext context;

  bool get _applies => layerOf(context) != Layer.outsideLib && layerOf(context) != Layer.generated;

  @override
  void visitMethodDeclaration(MethodDeclaration node) {
    if (!_applies || _isOverride(node)) return;
    if (_isWidget(node.returnType?.type)) rule.reportAtToken(node.name);
  }

  @override
  void visitFunctionDeclaration(FunctionDeclaration node) {
    if (!_applies) return;
    if (_isWidget(node.returnType?.type)) rule.reportAtToken(node.name);
  }
}

class FileLength extends _Rule {
  FileLength() : super(_code);

  static const _code = LintCode(
    'file_length',
    'A hand-written file must stay within $maxLines lines.',
    correctionMessage: 'Split it by what it does.',
  );

  @override
  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor) =>
      registry.addCompilationUnit(this, visitor);

  @override
  SimpleAstVisitor<void> visitor(RuleContext context) => _LengthVisitor(this, context);
}

class _LengthVisitor extends SimpleAstVisitor<void> {
  _LengthVisitor(this.rule, this.context);

  final AnalysisRule rule;
  final RuleContext context;

  @override
  void visitCompilationUnit(CompilationUnit node) {
    if (layerOf(context) == Layer.generated) return;

    // Counted from the line starts, as trailing comments are lines with no token.
    // A final newline starts a last line that no one wrote.
    final starts = node.lineInfo.lineStarts;
    final lines = starts.last == node.endToken.offset ? starts.length - 1 : starts.length;
    if (lines > maxLines) rule.reportAtToken(node.beginToken);
  }
}

class TimersInStateOrPlatform extends _Rule {
  TimersInStateOrPlatform() : super(_code);

  static const _code = LintCode(
    'timers_in_state_or_platform',
    'A timer lives only in state/ or platform/, and takes a named duration.',
    correctionMessage: 'Move it to a controller, and name the duration with its reason.',
  );

  @override
  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor) =>
      registry.addInstanceCreationExpression(this, visitor);

  @override
  SimpleAstVisitor<void> visitor(RuleContext context) => _TimerVisitor(this, context);
}

class _TimerVisitor extends SimpleAstVisitor<void> {
  _TimerVisitor(this.rule, this.context);

  final AnalysisRule rule;
  final RuleContext context;

  @override
  void visitInstanceCreationExpression(InstanceCreationExpression node) {
    final here = layerOf(context);
    if (here == Layer.outsideLib || here == Layer.generated) return;
    final constructor = node.constructorName;
    final owner = constructor.type.element?.name;
    final isTimer = owner == 'Timer' || (owner == 'Future' && constructor.name?.name == 'delayed');
    if (!isTimer) return;

    if (here != Layer.state && here != Layer.platform) {
      rule.reportAtNode(node);
      return;
    }
    final args = node.argumentList.arguments;
    if (args.isNotEmpty && _isLiteralDuration(args.first.argumentExpression)) rule.reportAtNode(args.first);
  }

  bool _isLiteralDuration(Expression expression) {
    if (expression is! InstanceCreationExpression) return false;
    if (expression.constructorName.type.element?.name != 'Duration') return false;
    return expression.argumentList.arguments.every((arg) => arg.argumentExpression is Literal);
  }
}

class CatchInControllers extends _Rule {
  CatchInControllers() : super(_code);

  static const _code = LintCode(
    'catch_in_controllers',
    'Only state/ and platform/ catch.',
    correctionMessage: 'Catch in a controller and turn the error into state.',
  );

  @override
  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor) {
    registry.addCatchClause(this, visitor);
    registry.addMethodInvocation(this, visitor);
  }

  @override
  SimpleAstVisitor<void> visitor(RuleContext context) => _CatchVisitor(this, context);
}

class _CatchVisitor extends SimpleAstVisitor<void> {
  _CatchVisitor(this.rule, this.context);

  final AnalysisRule rule;
  final RuleContext context;

  bool get _refused {
    final here = layerOf(context);
    return _isUi(here) || here == Layer.theme || here == Layer.root;
  }

  @override
  void visitCatchClause(CatchClause node) {
    if (_refused) rule.reportAtToken(node.catchKeyword ?? node.onKeyword!);
  }

  /// A future's `catchError` and `onError` catch as a `catch` clause does.
  @override
  void visitMethodInvocation(MethodInvocation node) {
    final name = node.methodName.name;
    if (name != 'catchError' && name != 'onError') return;
    if (node.methodName.element?.library?.uri.toString() != 'dart:async') return;
    if (_refused) rule.reportAtNode(node.methodName);
  }
}

class NoBang extends _Rule {
  NoBang() : super(_code);

  static const _code = LintCode(
    'no_bang',
    'Avoid `!`: narrow with a check or a pattern.',
    correctionMessage: 'If an invariant guarantees it, ignore this with the invariant as the reason.',
  );

  @override
  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor) =>
      registry.addPostfixExpression(this, visitor);

  @override
  SimpleAstVisitor<void> visitor(RuleContext context) => _BangVisitor(this, context);
}

class _BangVisitor extends SimpleAstVisitor<void> {
  _BangVisitor(this.rule, this.context);

  final AnalysisRule rule;
  final RuleContext context;

  @override
  void visitPostfixExpression(PostfixExpression node) {
    final here = layerOf(context);
    if (here == Layer.outsideLib || here == Layer.generated) return;
    if (node.operator.type == TokenType.BANG) rule.reportAtToken(node.operator);
  }
}

class SingleState extends _Rule {
  SingleState() : super(_code);

  static const _code = LintCode(
    'single_state',
    'A controller exposes one immutable state, not public mutable fields.',
    correctionMessage: 'Move this into the state class.',
  );

  @override
  void listen(RuleVisitorRegistry registry, SimpleAstVisitor<void> visitor) =>
      registry.addFieldDeclaration(this, visitor);

  @override
  SimpleAstVisitor<void> visitor(RuleContext context) => _StateVisitor(this, context);
}

class _StateVisitor extends SimpleAstVisitor<void> {
  _StateVisitor(this.rule, this.context);

  final AnalysisRule rule;
  final RuleContext context;

  @override
  void visitFieldDeclaration(FieldDeclaration node) {
    if (layerOf(context) != Layer.state || node.isStatic) return;
    if (node.fields.isFinal || node.fields.isConst) return;
    final owner = node.thisOrAncestorOfType<ClassDeclaration>();
    final Element? element = owner?.declaredFragment?.element;
    if (element is! InterfaceElement || !_extends(element, 'ChangeNotifier')) return;
    for (final field in node.fields.variables) {
      if (!field.name.lexeme.startsWith('_')) rule.reportAtToken(field.name);
    }
  }
}

/// Every rule the plugin has.
List<AnalysisRule> allRules() => [
  StateImportsNoUi(),
  UiNoPlatform(),
  UiNoBridgeCalls(),
  NoWidgetHelper(),
  FileLength(),
  TimersInStateOrPlatform(),
  CatchInControllers(),
  NoBang(),
  SingleState(),
];
