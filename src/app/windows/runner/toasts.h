#ifndef RUNNER_TOASTS_H_
#define RUNNER_TOASTS_H_

#include <cstdint>
#include <string>

// Takes the toast tagged |id| off for the app user model id |app|. The
// notification plugin removes a toast only for a packaged app, and this app
// is unpackaged, so an alert that ended would otherwise stay in the action
// centre. The plugin tags each toast with its id and sets no group. Whether
// Windows took it off: a toast already gone counts as taken off.
bool TakeOffToast(int64_t id, const std::string& app);

#endif  // RUNNER_TOASTS_H_
