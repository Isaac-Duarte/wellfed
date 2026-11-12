// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'server.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$ServerEvent {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerEvent);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerEvent()';
}


}

/// @nodoc
class $ServerEventCopyWith<$Res>  {
$ServerEventCopyWith(ServerEvent _, $Res Function(ServerEvent) __);
}


/// Adds pattern-matching-related methods to [ServerEvent].
extension ServerEventPatterns on ServerEvent {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ServerEvent_History value)?  history,TResult Function( ServerEvent_Connection value)?  connection,TResult Function( ServerEvent_Command value)?  command,TResult Function( ServerEvent_Install value)?  install,TResult Function( ServerEvent_Resync value)?  resync,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ServerEvent_History() when history != null:
return history(_that);case ServerEvent_Connection() when connection != null:
return connection(_that);case ServerEvent_Command() when command != null:
return command(_that);case ServerEvent_Install() when install != null:
return install(_that);case ServerEvent_Resync() when resync != null:
return resync(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ServerEvent_History value)  history,required TResult Function( ServerEvent_Connection value)  connection,required TResult Function( ServerEvent_Command value)  command,required TResult Function( ServerEvent_Install value)  install,required TResult Function( ServerEvent_Resync value)  resync,}){
final _that = this;
switch (_that) {
case ServerEvent_History():
return history(_that);case ServerEvent_Connection():
return connection(_that);case ServerEvent_Command():
return command(_that);case ServerEvent_Install():
return install(_that);case ServerEvent_Resync():
return resync(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ServerEvent_History value)?  history,TResult? Function( ServerEvent_Connection value)?  connection,TResult? Function( ServerEvent_Command value)?  command,TResult? Function( ServerEvent_Install value)?  install,TResult? Function( ServerEvent_Resync value)?  resync,}){
final _that = this;
switch (_that) {
case ServerEvent_History() when history != null:
return history(_that);case ServerEvent_Connection() when connection != null:
return connection(_that);case ServerEvent_Command() when command != null:
return command(_that);case ServerEvent_Install() when install != null:
return install(_that);case ServerEvent_Resync() when resync != null:
return resync(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String feeder,  ServerEntry entry)?  history,TResult Function( String feeder,  ServerConnection connection)?  connection,TResult Function( ServerCommand command)?  command,TResult Function( ServerInstall install)?  install,TResult Function()?  resync,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ServerEvent_History() when history != null:
return history(_that.feeder,_that.entry);case ServerEvent_Connection() when connection != null:
return connection(_that.feeder,_that.connection);case ServerEvent_Command() when command != null:
return command(_that.command);case ServerEvent_Install() when install != null:
return install(_that.install);case ServerEvent_Resync() when resync != null:
return resync();case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String feeder,  ServerEntry entry)  history,required TResult Function( String feeder,  ServerConnection connection)  connection,required TResult Function( ServerCommand command)  command,required TResult Function( ServerInstall install)  install,required TResult Function()  resync,}) {final _that = this;
switch (_that) {
case ServerEvent_History():
return history(_that.feeder,_that.entry);case ServerEvent_Connection():
return connection(_that.feeder,_that.connection);case ServerEvent_Command():
return command(_that.command);case ServerEvent_Install():
return install(_that.install);case ServerEvent_Resync():
return resync();}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String feeder,  ServerEntry entry)?  history,TResult? Function( String feeder,  ServerConnection connection)?  connection,TResult? Function( ServerCommand command)?  command,TResult? Function( ServerInstall install)?  install,TResult? Function()?  resync,}) {final _that = this;
switch (_that) {
case ServerEvent_History() when history != null:
return history(_that.feeder,_that.entry);case ServerEvent_Connection() when connection != null:
return connection(_that.feeder,_that.connection);case ServerEvent_Command() when command != null:
return command(_that.command);case ServerEvent_Install() when install != null:
return install(_that.install);case ServerEvent_Resync() when resync != null:
return resync();case _:
  return null;

}
}

}

/// @nodoc


class ServerEvent_History extends ServerEvent {
  const ServerEvent_History({required this.feeder, required this.entry}): super._();
  

 final  String feeder;
 final  ServerEntry entry;

/// Create a copy of ServerEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerEvent_HistoryCopyWith<ServerEvent_History> get copyWith => _$ServerEvent_HistoryCopyWithImpl<ServerEvent_History>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerEvent_History&&(identical(other.feeder, feeder) || other.feeder == feeder)&&(identical(other.entry, entry) || other.entry == entry));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feeder,entry);
}

@override
String toString() {
    return 'ServerEvent.history(feeder: $feeder, entry: $entry)';
}


}

/// @nodoc
abstract mixin class $ServerEvent_HistoryCopyWith<$Res> implements $ServerEventCopyWith<$Res> {
  factory $ServerEvent_HistoryCopyWith(ServerEvent_History value, $Res Function(ServerEvent_History) _then) = _$ServerEvent_HistoryCopyWithImpl;
@useResult
$Res call({
 String feeder, ServerEntry entry
});




}
/// @nodoc
class _$ServerEvent_HistoryCopyWithImpl<$Res>
    implements $ServerEvent_HistoryCopyWith<$Res> {
  _$ServerEvent_HistoryCopyWithImpl(this._self, this._then);

  final ServerEvent_History _self;
  final $Res Function(ServerEvent_History) _then;

/// Create a copy of ServerEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? feeder = null,Object? entry = null,}) {
  return _then(ServerEvent_History(
feeder: null == feeder ? _self.feeder : feeder // ignore: cast_nullable_to_non_nullable
as String,entry: null == entry ? _self.entry : entry // ignore: cast_nullable_to_non_nullable
as ServerEntry,
  ));
}


}

/// @nodoc


class ServerEvent_Connection extends ServerEvent {
  const ServerEvent_Connection({required this.feeder, required this.connection}): super._();
  

 final  String feeder;
 final  ServerConnection connection;

/// Create a copy of ServerEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerEvent_ConnectionCopyWith<ServerEvent_Connection> get copyWith => _$ServerEvent_ConnectionCopyWithImpl<ServerEvent_Connection>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerEvent_Connection&&(identical(other.feeder, feeder) || other.feeder == feeder)&&(identical(other.connection, connection) || other.connection == connection));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feeder,connection);
}

@override
String toString() {
    return 'ServerEvent.connection(feeder: $feeder, connection: $connection)';
}


}

/// @nodoc
abstract mixin class $ServerEvent_ConnectionCopyWith<$Res> implements $ServerEventCopyWith<$Res> {
  factory $ServerEvent_ConnectionCopyWith(ServerEvent_Connection value, $Res Function(ServerEvent_Connection) _then) = _$ServerEvent_ConnectionCopyWithImpl;
@useResult
$Res call({
 String feeder, ServerConnection connection
});




}
/// @nodoc
class _$ServerEvent_ConnectionCopyWithImpl<$Res>
    implements $ServerEvent_ConnectionCopyWith<$Res> {
  _$ServerEvent_ConnectionCopyWithImpl(this._self, this._then);

  final ServerEvent_Connection _self;
  final $Res Function(ServerEvent_Connection) _then;

/// Create a copy of ServerEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? feeder = null,Object? connection = null,}) {
  return _then(ServerEvent_Connection(
feeder: null == feeder ? _self.feeder : feeder // ignore: cast_nullable_to_non_nullable
as String,connection: null == connection ? _self.connection : connection // ignore: cast_nullable_to_non_nullable
as ServerConnection,
  ));
}


}

/// @nodoc


class ServerEvent_Command extends ServerEvent {
  const ServerEvent_Command({required this.command}): super._();
  

 final  ServerCommand command;

/// Create a copy of ServerEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerEvent_CommandCopyWith<ServerEvent_Command> get copyWith => _$ServerEvent_CommandCopyWithImpl<ServerEvent_Command>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerEvent_Command&&(identical(other.command, command) || other.command == command));
}


@override
int get hashCode {
    return Object.hash(runtimeType,command);
}

@override
String toString() {
    return 'ServerEvent.command(command: $command)';
}


}

/// @nodoc
abstract mixin class $ServerEvent_CommandCopyWith<$Res> implements $ServerEventCopyWith<$Res> {
  factory $ServerEvent_CommandCopyWith(ServerEvent_Command value, $Res Function(ServerEvent_Command) _then) = _$ServerEvent_CommandCopyWithImpl;
@useResult
$Res call({
 ServerCommand command
});




}
/// @nodoc
class _$ServerEvent_CommandCopyWithImpl<$Res>
    implements $ServerEvent_CommandCopyWith<$Res> {
  _$ServerEvent_CommandCopyWithImpl(this._self, this._then);

  final ServerEvent_Command _self;
  final $Res Function(ServerEvent_Command) _then;

/// Create a copy of ServerEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? command = null,}) {
  return _then(ServerEvent_Command(
command: null == command ? _self.command : command // ignore: cast_nullable_to_non_nullable
as ServerCommand,
  ));
}


}

/// @nodoc


class ServerEvent_Install extends ServerEvent {
  const ServerEvent_Install({required this.install}): super._();
  

 final  ServerInstall install;

/// Create a copy of ServerEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerEvent_InstallCopyWith<ServerEvent_Install> get copyWith => _$ServerEvent_InstallCopyWithImpl<ServerEvent_Install>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerEvent_Install&&(identical(other.install, install) || other.install == install));
}


@override
int get hashCode {
    return Object.hash(runtimeType,install);
}

@override
String toString() {
    return 'ServerEvent.install(install: $install)';
}


}

/// @nodoc
abstract mixin class $ServerEvent_InstallCopyWith<$Res> implements $ServerEventCopyWith<$Res> {
  factory $ServerEvent_InstallCopyWith(ServerEvent_Install value, $Res Function(ServerEvent_Install) _then) = _$ServerEvent_InstallCopyWithImpl;
@useResult
$Res call({
 ServerInstall install
});




}
/// @nodoc
class _$ServerEvent_InstallCopyWithImpl<$Res>
    implements $ServerEvent_InstallCopyWith<$Res> {
  _$ServerEvent_InstallCopyWithImpl(this._self, this._then);

  final ServerEvent_Install _self;
  final $Res Function(ServerEvent_Install) _then;

/// Create a copy of ServerEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? install = null,}) {
  return _then(ServerEvent_Install(
install: null == install ? _self.install : install // ignore: cast_nullable_to_non_nullable
as ServerInstall,
  ));
}


}

/// @nodoc


class ServerEvent_Resync extends ServerEvent {
  const ServerEvent_Resync(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerEvent_Resync);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerEvent.resync()';
}


}




/// @nodoc
mixin _$ServerUpdate {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerUpdate);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerUpdate()';
}


}

/// @nodoc
class $ServerUpdateCopyWith<$Res>  {
$ServerUpdateCopyWith(ServerUpdate _, $Res Function(ServerUpdate) __);
}


/// Adds pattern-matching-related methods to [ServerUpdate].
extension ServerUpdatePatterns on ServerUpdate {
/// A variant of `map` that fallback to returning `orElse`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ServerUpdate_Opened value)?  opened,TResult Function( ServerUpdate_Event value)?  event,TResult Function( ServerUpdate_Closed value)?  closed,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ServerUpdate_Opened() when opened != null:
return opened(_that);case ServerUpdate_Event() when event != null:
return event(_that);case ServerUpdate_Closed() when closed != null:
return closed(_that);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// Callbacks receives the raw object, upcasted.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case final Subclass2 value:
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ServerUpdate_Opened value)  opened,required TResult Function( ServerUpdate_Event value)  event,required TResult Function( ServerUpdate_Closed value)  closed,}){
final _that = this;
switch (_that) {
case ServerUpdate_Opened():
return opened(_that);case ServerUpdate_Event():
return event(_that);case ServerUpdate_Closed():
return closed(_that);}
}
/// A variant of `map` that fallback to returning `null`.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case final Subclass value:
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ServerUpdate_Opened value)?  opened,TResult? Function( ServerUpdate_Event value)?  event,TResult? Function( ServerUpdate_Closed value)?  closed,}){
final _that = this;
switch (_that) {
case ServerUpdate_Opened() when opened != null:
return opened(_that);case ServerUpdate_Event() when event != null:
return event(_that);case ServerUpdate_Closed() when closed != null:
return closed(_that);case _:
  return null;

}
}
/// A variant of `when` that fallback to an `orElse` callback.
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return orElse();
/// }
/// ```

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  opened,TResult Function( String resume,  ServerEvent event)?  event,TResult Function( ServerError? error)?  closed,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ServerUpdate_Opened() when opened != null:
return opened();case ServerUpdate_Event() when event != null:
return event(_that.resume,_that.event);case ServerUpdate_Closed() when closed != null:
return closed(_that.error);case _:
  return orElse();

}
}
/// A `switch`-like method, using callbacks.
///
/// As opposed to `map`, this offers destructuring.
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case Subclass2(:final field2):
///     return ...;
/// }
/// ```

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  opened,required TResult Function( String resume,  ServerEvent event)  event,required TResult Function( ServerError? error)  closed,}) {final _that = this;
switch (_that) {
case ServerUpdate_Opened():
return opened();case ServerUpdate_Event():
return event(_that.resume,_that.event);case ServerUpdate_Closed():
return closed(_that.error);}
}
/// A variant of `when` that fallback to returning `null`
///
/// It is equivalent to doing:
/// ```dart
/// switch (sealedClass) {
///   case Subclass(:final field):
///     return ...;
///   case _:
///     return null;
/// }
/// ```

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  opened,TResult? Function( String resume,  ServerEvent event)?  event,TResult? Function( ServerError? error)?  closed,}) {final _that = this;
switch (_that) {
case ServerUpdate_Opened() when opened != null:
return opened();case ServerUpdate_Event() when event != null:
return event(_that.resume,_that.event);case ServerUpdate_Closed() when closed != null:
return closed(_that.error);case _:
  return null;

}
}

}

/// @nodoc


class ServerUpdate_Opened extends ServerUpdate {
  const ServerUpdate_Opened(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerUpdate_Opened);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerUpdate.opened()';
}


}




/// @nodoc


class ServerUpdate_Event extends ServerUpdate {
  const ServerUpdate_Event({required this.resume, required this.event}): super._();
  

 final  String resume;
 final  ServerEvent event;

/// Create a copy of ServerUpdate
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerUpdate_EventCopyWith<ServerUpdate_Event> get copyWith => _$ServerUpdate_EventCopyWithImpl<ServerUpdate_Event>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerUpdate_Event&&(identical(other.resume, resume) || other.resume == resume)&&(identical(other.event, event) || other.event == event));
}


@override
int get hashCode {
    return Object.hash(runtimeType,resume,event);
}

@override
String toString() {
    return 'ServerUpdate.event(resume: $resume, event: $event)';
}


}

/// @nodoc
abstract mixin class $ServerUpdate_EventCopyWith<$Res> implements $ServerUpdateCopyWith<$Res> {
  factory $ServerUpdate_EventCopyWith(ServerUpdate_Event value, $Res Function(ServerUpdate_Event) _then) = _$ServerUpdate_EventCopyWithImpl;
@useResult
$Res call({
 String resume, ServerEvent event
});


$ServerEventCopyWith<$Res> get event;

}
/// @nodoc
class _$ServerUpdate_EventCopyWithImpl<$Res>
    implements $ServerUpdate_EventCopyWith<$Res> {
  _$ServerUpdate_EventCopyWithImpl(this._self, this._then);

  final ServerUpdate_Event _self;
  final $Res Function(ServerUpdate_Event) _then;

/// Create a copy of ServerUpdate
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? resume = null,Object? event = null,}) {
  return _then(ServerUpdate_Event(
resume: null == resume ? _self.resume : resume // ignore: cast_nullable_to_non_nullable
as String,event: null == event ? _self.event : event // ignore: cast_nullable_to_non_nullable
as ServerEvent,
  ));
}

/// Create a copy of ServerUpdate
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$ServerEventCopyWith<$Res> get event {
  
  return $ServerEventCopyWith<$Res>(_self.event, (value) {
    return _then(_self.copyWith(event: value));
  });
}
}

/// @nodoc


class ServerUpdate_Closed extends ServerUpdate {
  const ServerUpdate_Closed({this.error}): super._();
  

 final  ServerError? error;

/// Create a copy of ServerUpdate
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerUpdate_ClosedCopyWith<ServerUpdate_Closed> get copyWith => _$ServerUpdate_ClosedCopyWithImpl<ServerUpdate_Closed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerUpdate_Closed&&(identical(other.error, error) || other.error == error));
}


@override
int get hashCode {
    return Object.hash(runtimeType,error);
}

@override
String toString() {
    return 'ServerUpdate.closed(error: $error)';
}


}

/// @nodoc
abstract mixin class $ServerUpdate_ClosedCopyWith<$Res> implements $ServerUpdateCopyWith<$Res> {
  factory $ServerUpdate_ClosedCopyWith(ServerUpdate_Closed value, $Res Function(ServerUpdate_Closed) _then) = _$ServerUpdate_ClosedCopyWithImpl;
@useResult
$Res call({
 ServerError? error
});




}
/// @nodoc
class _$ServerUpdate_ClosedCopyWithImpl<$Res>
    implements $ServerUpdate_ClosedCopyWith<$Res> {
  _$ServerUpdate_ClosedCopyWithImpl(this._self, this._then);

  final ServerUpdate_Closed _self;
  final $Res Function(ServerUpdate_Closed) _then;

/// Create a copy of ServerUpdate
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? error = freezed,}) {
  return _then(ServerUpdate_Closed(
error: freezed == error ? _self.error : error // ignore: cast_nullable_to_non_nullable
as ServerError?,
  ));
}


}

// dart format on
