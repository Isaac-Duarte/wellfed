# Builds wellfed-app-bridge for the macOS desktop app with cargokit, as a
# static library linked into the runner, as the Linux side bundles a shared
# one beside it.
Pod::Spec.new do |s|
  s.name             = 'wellfed_app_bridge'
  s.version          = '0.0.1'
  s.summary          = 'The app bridge, built by cargokit.'
  s.homepage         = 'https://github.com/Isaac-Duarte/wellfed'
  s.license          = { :type => 'MIT' }
  s.author           = 'Wellfed'

  # CocoaPods makes a framework only for a pod with a source file.
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'FlutterMacOS'

  s.platform = :osx, '12.0'
  s.swift_version = '5.0'

  s.script_phase = {
    :name => 'Build Rust library',
    # The crate's folder relative to this one, and its library's name.
    :script => 'sh "$PODS_TARGET_SRCROOT/../cargokit/build_pod.sh" ../../rust wellfed_app_bridge',
    :execution_position => :before_compile,
    :input_files => ['${BUILT_PRODUCTS_DIR}/cargokit_phony'],
    # Tells Xcode the -force_load below names what this step makes. Cargokit
    # names the library after the package, hyphens and all.
    :output_files => ["${BUILT_PRODUCTS_DIR}/libwellfed-app-bridge.a"],
  }
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Every symbol stays: Dart looks them up at run time, so the linker would
    # otherwise drop them all.
    'OTHER_LDFLAGS' => '-force_load ${BUILT_PRODUCTS_DIR}/libwellfed-app-bridge.a',
  }
end
