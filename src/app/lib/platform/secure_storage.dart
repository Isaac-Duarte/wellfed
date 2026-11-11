import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Where this device keeps its secrets: Android's Keystore, libsecret on
/// Linux, and the login keychain on macOS.
///
/// The Mac build is signed ad hoc, with no team ID, so it cannot hold the
/// keychain access group the data protection keychain needs: a write there
/// fails with errSecMissingEntitlement (-34018). The login keychain needs no
/// entitlement, and its items are this app's through their access list.
const FlutterSecureStorage secureStorage = FlutterSecureStorage(
  mOptions: MacOsOptions(usesDataProtectionKeychain: false),
);

/// What secure storage calls this phone's client secret.
const clientSecretName = 'client-secret';
