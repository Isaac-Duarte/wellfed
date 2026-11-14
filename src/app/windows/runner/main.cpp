#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>

#include "flutter_window.h"
#include "utils.h"

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  // One Wellfed per user session: a second would follow the same feeders
  // and post every alert twice, over the same stores. Opening it again
  // brings the running one forward instead. A debug build has a name of its
  // own, so `flutter run` starts beside an installed Wellfed rather than
  // raising it.
#ifdef _DEBUG
  const wchar_t* const single_name =
      L"Local\\io.github.isaacduarte.wellfed.debug";
#else
  const wchar_t* const single_name = L"Local\\io.github.isaacduarte.wellfed";
#endif
  HANDLE single = ::CreateMutexW(nullptr, FALSE, single_name);
  if (single != nullptr && ::GetLastError() == ERROR_ALREADY_EXISTS) {
    // The first may still be making its window: look for up to 2 s.
    HWND running = nullptr;
    for (int tries = 0; running == nullptr && tries < 20; ++tries) {
      running = ::FindWindowW(L"FLUTTER_RUNNER_WIN32_WINDOW", L"Wellfed");
      if (running == nullptr) {
        ::Sleep(100);
      }
    }
    if (running != nullptr) {
      if (::IsIconic(running)) {
        ::ShowWindow(running, SW_RESTORE);
      }
      ::SetForegroundWindow(running);
    }
    ::CloseHandle(single);
    return EXIT_SUCCESS;
  }

  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"data");

  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1280, 720);
  if (!window.Create(L"Wellfed", origin, size)) {
    return EXIT_FAILURE;
  }
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  if (single != nullptr) {
    ::CloseHandle(single);
  }
  return EXIT_SUCCESS;
}
