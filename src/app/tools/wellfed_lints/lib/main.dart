import 'package:analysis_server_plugin/plugin.dart';
import 'package:analysis_server_plugin/registry.dart';
import 'package:wellfed_lints/src/rules.dart';

/// The instance the analysis server loads.
final plugin = WellfedLints();

/// The architecture rules of `docs/development/dart.md`. They are warnings, so
/// they are on wherever the plugin is, and no rule can be left out by mistake.
class WellfedLints extends Plugin {
  @override
  String get name => 'wellfed_lints';

  @override
  void register(PluginRegistry registry) {
    allRules().forEach(registry.registerWarningRule);
  }
}
