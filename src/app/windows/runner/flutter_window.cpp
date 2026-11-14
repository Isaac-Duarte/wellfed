#include "flutter_window.h"

#include <flutter/standard_method_codec.h>

#include <optional>
#include <string>

#include "flutter/generated_plugin_registrant.h"
#include "toasts.h"

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());
  SetChildContent(flutter_controller_->view()->GetNativeWindow());
  RegisterChannels();

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    this->Show();
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

void FlutterWindow::RegisterChannels() {
  flutter::BinaryMessenger* messenger =
      flutter_controller_->engine()->messenger();
  const auto& codec = flutter::StandardMethodCodec::GetInstance();

  window_channel_ =
      std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
          messenger, "io.github.isaacduarte.wellfed/window", &codec);
  window_channel_->SetMethodCallHandler([this](const auto& call, auto result) {
    if (call.method_name() != "front") {
      result->NotImplemented();
      return;
    }
    // Windows may refuse the foreground to a process the owner isn't using;
    // a toast's click usually grants it. Refused, the taskbar button
    // flashes instead.
    HWND window = GetHandle();
    if (::IsIconic(window)) {
      ::ShowWindow(window, SW_RESTORE);
    }
    ::SetForegroundWindow(window);
    result->Success();
  });

  toast_channel_ =
      std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
          messenger, "io.github.isaacduarte.wellfed/toasts", &codec);
  toast_channel_->SetMethodCallHandler([](const auto& call, auto result) {
    const auto* arguments =
        std::get_if<flutter::EncodableMap>(call.arguments());
    const auto app = arguments == nullptr
                         ? flutter::EncodableMap::const_iterator()
                         : arguments->find(flutter::EncodableValue("app"));
    const auto* name = arguments == nullptr || app == arguments->end()
                           ? nullptr
                           : std::get_if<std::string>(&app->second);
    if (name == nullptr) {
      result->Error("arguments", "a toast call needs the app's id");
      return;
    }

    const std::string& method = call.method_name();
    if (method == "switches") {
      const ToastSwitches switches = ReadToastSwitches(*name);
      flutter::EncodableMap read;
      if (switches.app) {
        read[flutter::EncodableValue("app")] =
            flutter::EncodableValue(static_cast<int64_t>(*switches.app));
      }
      if (switches.all) {
        read[flutter::EncodableValue("all")] =
            flutter::EncodableValue(static_cast<int64_t>(*switches.all));
      }
      result->Success(flutter::EncodableValue(read));
    } else if (method == "shown") {
      flutter::EncodableList ids;
      for (const int64_t id : ShownToasts(*name)) {
        ids.push_back(flutter::EncodableValue(id));
      }
      result->Success(flutter::EncodableValue(ids));
    } else if (method == "takeOff") {
      const auto id = arguments->find(flutter::EncodableValue("id"));
      if (id == arguments->end()) {
        result->Error("arguments", "takeOff needs an id");
        return;
      }
      result->Success(flutter::EncodableValue(
          TakeOffToast(id->second.LongValue(), *name)));
    } else {
      result->NotImplemented();
    }
  });
}

void FlutterWindow::OnDestroy() {
  window_channel_ = nullptr;
  toast_channel_ = nullptr;
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
