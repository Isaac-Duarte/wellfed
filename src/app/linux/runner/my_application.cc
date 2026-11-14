#include "my_application.h"

#include <flutter_linux/flutter_linux.h>
#include <string.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include "flutter/generated_plugin_registrant.h"

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
  // Brings the window forward when an alert's notification is clicked
  // (`lib/platform/app_window.dart`).
  FlMethodChannel* window_channel;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// Called when first Flutter frame received.
static void first_frame_cb(MyApplication* self, FlView* view) {
  gtk_widget_show(gtk_widget_get_toplevel(GTK_WIDGET(view)));
}

// The window channel's one call, "front": the window restored and raised.
// A Wayland compositor may only mark it as wanting attention, since the
// notification server hands the app no activation token through the plugin.
static void window_call_cb(FlMethodChannel* channel, FlMethodCall* call,
                           gpointer user_data) {
  GtkWindow* window = GTK_WINDOW(user_data);
  g_autoptr(FlMethodResponse) response = nullptr;
  if (strcmp(fl_method_call_get_name(call), "front") == 0) {
    gtk_window_deiconify(window);
    gtk_window_present(window);
    response = FL_METHOD_RESPONSE(fl_method_success_response_new(nullptr));
  } else {
    response = FL_METHOD_RESPONSE(fl_method_not_implemented_response_new());
  }
  fl_method_call_respond(call, response, nullptr);
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);

  // Opened again: GApplication hands the second launch's activation to this
  // one, which brings its window forward rather than opening another.
  GList* windows = gtk_application_get_windows(GTK_APPLICATION(application));
  if (windows != nullptr) {
    gtk_window_present(GTK_WINDOW(windows->data));
    return;
  }

  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));

  // GNOME draws its own title bars, so a header bar matches it there.
  // Everywhere else (KDE, tiling window managers) the compositor draws the
  // frame, and a GTK header bar would look foreign or double it up.
  const gchar* desktop = g_getenv("XDG_CURRENT_DESKTOP");
  gboolean use_header_bar =
      desktop != nullptr && strstr(desktop, "GNOME") != nullptr;
#ifdef GDK_WINDOWING_X11
  GdkScreen* screen = gtk_window_get_screen(window);
  if (GDK_IS_X11_SCREEN(screen)) {
    const gchar* wm_name = gdk_x11_screen_get_window_manager_name(screen);
    use_header_bar = g_strcmp0(wm_name, "GNOME Shell") == 0;
  }
#endif
  if (use_header_bar) {
    GtkHeaderBar* header_bar = GTK_HEADER_BAR(gtk_header_bar_new());
    gtk_widget_show(GTK_WIDGET(header_bar));
    gtk_header_bar_set_title(header_bar, "Wellfed");
    gtk_header_bar_set_show_close_button(header_bar, TRUE);
    gtk_window_set_titlebar(window, GTK_WIDGET(header_bar));
  } else {
    gtk_window_set_title(window, "Wellfed");
  }

  // Room for the sidebar beside a page; below this the page's two columns
  // already fold into one.
  gtk_window_set_default_size(window, 1200, 800);
  GdkGeometry least = {};
  least.min_width = 760;
  least.min_height = 560;
  gtk_window_set_geometry_hints(window, nullptr, &least, GDK_HINT_MIN_SIZE);

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  // The page color of the theme the desktop asks for, so a resize shows the
  // page rather than black: tokens.json's light and dark `background`. GNOME
  // 42 and later say dark through the color-scheme setting, others through
  // GTK's prefer-dark or a dark theme's name. An appearance chosen in the
  // app's settings can still differ until Flutter paints.
  gboolean dark = FALSE;
  GSettingsSchemaSource* schemas = g_settings_schema_source_get_default();
  g_autoptr(GSettingsSchema) gnome_interface =
      schemas == nullptr ? nullptr
                         : g_settings_schema_source_lookup(
                               schemas, "org.gnome.desktop.interface", TRUE);
  if (gnome_interface != nullptr &&
      g_settings_schema_has_key(gnome_interface, "color-scheme")) {
    g_autoptr(GSettings) settings =
        g_settings_new("org.gnome.desktop.interface");
    g_autofree gchar* scheme = g_settings_get_string(settings, "color-scheme");
    dark = g_strcmp0(scheme, "prefer-dark") == 0;
  }
  gboolean prefer_dark = FALSE;
  g_autofree gchar* theme_name = nullptr;
  g_object_get(gtk_settings_get_default(), "gtk-application-prefer-dark-theme",
               &prefer_dark, "gtk-theme-name", &theme_name, nullptr);
  g_autofree gchar* theme_lower =
      theme_name == nullptr ? nullptr : g_ascii_strdown(theme_name, -1);
  dark = dark || prefer_dark ||
         (theme_lower != nullptr && strstr(theme_lower, "dark") != nullptr);
  GdkRGBA background_color;
  gdk_rgba_parse(&background_color, dark ? "#121417" : "#F4F6FA");
  fl_view_set_background_color(view, &background_color);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  // Show the window when Flutter renders.
  // Requires the view to be realized so we can start rendering.
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));

  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  self->window_channel = fl_method_channel_new(
      fl_engine_get_binary_messenger(fl_view_get_engine(view)),
      "io.github.isaacduarte.wellfed/window", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(self->window_channel,
                                            window_call_cb, window, nullptr);

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

// Implements GApplication::local_command_line.
static gboolean my_application_local_command_line(GApplication* application,
                                                  gchar*** arguments,
                                                  int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  // Strip out the first argument as it is the binary name.
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);

  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
    g_warning("Failed to register: %s", error->message);
    *exit_status = 1;
    return TRUE;
  }

  g_application_activate(application);
  *exit_status = 0;

  return TRUE;
}

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application startup.

  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application shutdown.

  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

// Implements GObject::dispose.
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  g_clear_object(&self->window_channel);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->local_command_line =
      my_application_local_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {}

MyApplication* my_application_new() {
  // Set the program name to the application ID, which helps various systems
  // like GTK and desktop environments map this running application to its
  // corresponding .desktop file. This ensures better integration by allowing
  // the application to be recognized beyond its binary name.
  g_set_prgname(APPLICATION_ID);

  // Unique per user session: a second Wellfed would follow the same feeders
  // and post every alert twice, over the same stores. A debug build is not,
  // so `flutter run` starts beside an installed Wellfed rather than raising
  // it. It keeps the same id, which path_provider names the app's folder by.
#ifndef NDEBUG
  const GApplicationFlags flags = G_APPLICATION_NON_UNIQUE;
#elif GLIB_CHECK_VERSION(2, 74, 0)
  const GApplicationFlags flags = G_APPLICATION_DEFAULT_FLAGS;
#else
  const GApplicationFlags flags = G_APPLICATION_FLAGS_NONE;
#endif
  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID, "flags",
                                     flags, nullptr));
}
