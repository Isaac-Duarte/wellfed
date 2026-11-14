#include "toasts.h"

#include <windows.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.UI.Notifications.h>

#include <cwchar>
#include <string>

using winrt::Windows::UI::Notifications::ToastNotificationManager;

namespace {

// The DWORD |value| under HKEY_CURRENT_USER\|key|, if it is there.
std::optional<uint32_t> ReadDword(const std::wstring& key,
                                  const wchar_t* value) {
  DWORD data = 0;
  DWORD size = sizeof(data);
  const LSTATUS status =
      ::RegGetValueW(HKEY_CURRENT_USER, key.c_str(), value, RRF_RT_REG_DWORD,
                     nullptr, &data, &size);
  if (status != ERROR_SUCCESS) {
    return std::nullopt;
  }
  return static_cast<uint32_t>(data);
}

}  // namespace

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

ToastSwitches ReadToastSwitches(const std::string& app) {
  ToastSwitches switches;
  switches.app = ReadDword(
      L"Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings\\" +
          std::wstring(winrt::to_hstring(app)),
      L"Enabled");
  switches.all = ReadDword(
      L"Software\\Microsoft\\Windows\\CurrentVersion\\PushNotifications",
      L"ToastEnabled");
  return switches;
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
