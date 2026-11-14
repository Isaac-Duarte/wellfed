#ifndef RUNNER_TOASTS_H_
#define RUNNER_TOASTS_H_

#include <cstdint>
#include <optional>
#include <string>
#include <vector>

// The toast calls the notification plugin makes only for a packaged app,
// for this unpackaged one under its app user model id |app|. The plugin
// tags each toast with its id and sets no group. None of these throws: a
// WinRT failure reads as the safe answer each names.

// Takes the toast tagged |id| off. Whether Windows took it off: a toast
// already gone counts as taken off.
bool TakeOffToast(int64_t id, const std::string& app);

// The two switches the Settings app writes for toasts: the app's own
// (`Enabled` under Notifications\Settings\<app>) and every app's
// (`ToastEnabled` under PushNotifications). 0 means off; an absent or
// unreadable value is empty, which Windows reads as on. Dart decides from
// them: a notifier's own setting can't be read for an unpackaged app id,
// which Windows answers with "element not found".
struct ToastSwitches {
  std::optional<uint32_t> app;
  std::optional<uint32_t> all;
};
ToastSwitches ReadToastSwitches(const std::string& app);

// The ids of the app's toasts in the action centre. None when Windows
// can't say.
std::vector<int64_t> ShownToasts(const std::string& app);

#endif  // RUNNER_TOASTS_H_
