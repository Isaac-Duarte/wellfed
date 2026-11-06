# Builds wellfed-app-bridge for the Linux desktop app with cargokit, and
# bundles the library beside the runner, as the Android side puts it in the APK.
cmake_minimum_required(VERSION 3.10)

set(PROJECT_NAME "wellfed_app_bridge")
project(${PROJECT_NAME} LANGUAGES CXX)

include("../cargokit/cmake/cargokit.cmake")
apply_cargokit(${PROJECT_NAME} ../../rust wellfed_app_bridge "")

set(
  wellfed_app_bridge_bundled_libraries
  "${${PROJECT_NAME}_cargokit_lib}"
  PARENT_SCOPE
)
