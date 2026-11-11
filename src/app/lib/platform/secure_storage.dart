import 'package:flutter_secure_storage/flutter_secure_storage.dart';

/// Where this device keeps its secrets: Android's Keystore, libsecret on
/// Linux, and the login keychain on macOS.
///
/// The Mac build is signed ad hoc, with no team ID, so it cannot hold the
/// keychain access group the data protection keychain needs: a write there
/// fails with errSecMissingEntitlement (-34018). The login keychain needs no
/// entitlement, and its items are this app's through their access list.
///
/// On Android a read that fails throws rather than deleting the entry: the
/// plugin's default `resetOnError` would erase the client key on a passing
/// Keystore fault, and the app would read that as a first start and make a
/// new key no feeder knows. A failed read stops the app's start instead,
/// with the key left where it is for the next one.
const FlutterSecureStorage secureStorage = FlutterSecureStorage(
  aOptions: AndroidOptions(resetOnError: false),
  mOptions: MacOsOptions(usesDataProtectionKeychain: false),
);

/// What secure storage calls this phone's client secret.
const clientSecretName = 'client-secret';
