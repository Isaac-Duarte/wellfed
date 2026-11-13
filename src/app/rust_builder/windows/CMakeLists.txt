# Builds wellfed-app-bridge for the Windows desktop app with cargokit, and
# bundles the DLL beside the runner. CMake resolves ".." from the plugin link
# under windows/flutter/ephemeral/.plugin_symlinks as text, so the path to
# ../rust climbs from there to the app.
cmake_minimum_required(VERSION 3.14)

set(PROJECT_NAME "wellfed_app_bridge")
project(${PROJECT_NAME} LANGUAGES CXX)

include("../cargokit/cmake/cargokit.cmake")
apply_cargokit(${PROJECT_NAME} ../../../../../../rust wellfed_app_bridge "")

set(
  wellfed_app_bridge_bundled_libraries
  "${${PROJECT_NAME}_cargokit_lib}"
  PARENT_SCOPE
)
