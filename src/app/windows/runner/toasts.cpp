#include "toasts.h"

#include <windows.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Notifications.h>

#include <cwchar>

using winrt::Windows::UI::Notifications::NotificationSetting;
using winrt::Windows::UI::Notifications::ToastNotificationManager;

bool TakeOffToast(int64_t id, const std::string& app) {
  try {
    // The id-and-group form an unpackaged app must use: the shorter forms
    // look for the calling package, which this app has none of. The group
    // is empty, as the plugin sets none.
    ToastNotificationManager::History().Remove(winrt::to_hstring(id), L"",
                                               winrt::to_hstring(app));
    return true;
  } catch (winrt::hresult_error const&) {
    return false;
  }
}

bool ToastsEnabled(const std::string& app) {
  try {
    return ToastNotificationManager::CreateToastNotifier(winrt::to_hstring(app))
               .Setting() == NotificationSetting::Enabled;
  } catch (winrt::hresult_error const&) {
    return false;
  }
}

std::vector<int64_t> ShownToasts(const std::string& app) {
  std::vector<int64_t> ids;
  try {
    for (const auto& toast :
         ToastNotificationManager::History().GetHistory(winrt::to_hstring(app))) {
      const winrt::hstring tag = toast.Tag();
      wchar_t* end = nullptr;
      const long long id = std::wcstoll(tag.c_str(), &end, 10);
      // A tag that isn't a whole number is no alert's.
      if (!tag.empty() && end != nullptr && *end == L'\0') ids.push_back(id);
    }
  } catch (winrt::hresult_error const&) {
    ids.clear();
  }
  return ids;
}
