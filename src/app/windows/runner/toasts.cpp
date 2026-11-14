#include "toasts.h"

#include <windows.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Notifications.h>

bool TakeOffToast(int64_t id, const std::string& app) {
  try {
    // The id-and-group form an unpackaged app must use: the shorter forms
    // look for the calling package, which this app has none of. The group
    // is empty, as the plugin sets none.
    winrt::Windows::UI::Notifications::ToastNotificationManager::History()
        .Remove(winrt::to_hstring(id), L"", winrt::to_hstring(app));
    return true;
  } catch (winrt::hresult_error const&) {
    return false;
  }
}
