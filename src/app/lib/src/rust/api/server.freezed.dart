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
mixin _$ServerCommandOutcome {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerCommandOutcome()';
}


}

/// @nodoc
class $ServerCommandOutcomeCopyWith<$Res>  {
$ServerCommandOutcomeCopyWith(ServerCommandOutcome _, $Res Function(ServerCommandOutcome) __);
}


/// Adds pattern-matching-related methods to [ServerCommandOutcome].
extension ServerCommandOutcomePatterns on ServerCommandOutcome {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ServerCommandOutcome_Done value)?  done,TResult Function( ServerCommandOutcome_Settings value)?  settings,TResult Function( ServerCommandOutcome_Plans value)?  plans,TResult Function( ServerCommandOutcome_Calibration value)?  calibration,TResult Function( ServerCommandOutcome_Call value)?  call,TResult Function( ServerCommandOutcome_Superseded value)?  superseded,TResult Function( ServerCommandOutcome_Refused value)?  refused,TResult Function( ServerCommandOutcome_Invited value)?  invited,TResult Function( ServerCommandOutcome_Feeding value)?  feeding,TResult Function( ServerCommandOutcome_Fed value)?  fed,TResult Function( ServerCommandOutcome_Unrecorded value)?  unrecorded,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ServerCommandOutcome_Done() when done != null:
return done(_that);case ServerCommandOutcome_Settings() when settings != null:
return settings(_that);case ServerCommandOutcome_Plans() when plans != null:
return plans(_that);case ServerCommandOutcome_Calibration() when calibration != null:
return calibration(_that);case ServerCommandOutcome_Call() when call != null:
return call(_that);case ServerCommandOutcome_Superseded() when superseded != null:
return superseded(_that);case ServerCommandOutcome_Refused() when refused != null:
return refused(_that);case ServerCommandOutcome_Invited() when invited != null:
return invited(_that);case ServerCommandOutcome_Feeding() when feeding != null:
return feeding(_that);case ServerCommandOutcome_Fed() when fed != null:
return fed(_that);case ServerCommandOutcome_Unrecorded() when unrecorded != null:
return unrecorded(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ServerCommandOutcome_Done value)  done,required TResult Function( ServerCommandOutcome_Settings value)  settings,required TResult Function( ServerCommandOutcome_Plans value)  plans,required TResult Function( ServerCommandOutcome_Calibration value)  calibration,required TResult Function( ServerCommandOutcome_Call value)  call,required TResult Function( ServerCommandOutcome_Superseded value)  superseded,required TResult Function( ServerCommandOutcome_Refused value)  refused,required TResult Function( ServerCommandOutcome_Invited value)  invited,required TResult Function( ServerCommandOutcome_Feeding value)  feeding,required TResult Function( ServerCommandOutcome_Fed value)  fed,required TResult Function( ServerCommandOutcome_Unrecorded value)  unrecorded,}){
final _that = this;
switch (_that) {
case ServerCommandOutcome_Done():
return done(_that);case ServerCommandOutcome_Settings():
return settings(_that);case ServerCommandOutcome_Plans():
return plans(_that);case ServerCommandOutcome_Calibration():
return calibration(_that);case ServerCommandOutcome_Call():
return call(_that);case ServerCommandOutcome_Superseded():
return superseded(_that);case ServerCommandOutcome_Refused():
return refused(_that);case ServerCommandOutcome_Invited():
return invited(_that);case ServerCommandOutcome_Feeding():
return feeding(_that);case ServerCommandOutcome_Fed():
return fed(_that);case ServerCommandOutcome_Unrecorded():
return unrecorded(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ServerCommandOutcome_Done value)?  done,TResult? Function( ServerCommandOutcome_Settings value)?  settings,TResult? Function( ServerCommandOutcome_Plans value)?  plans,TResult? Function( ServerCommandOutcome_Calibration value)?  calibration,TResult? Function( ServerCommandOutcome_Call value)?  call,TResult? Function( ServerCommandOutcome_Superseded value)?  superseded,TResult? Function( ServerCommandOutcome_Refused value)?  refused,TResult? Function( ServerCommandOutcome_Invited value)?  invited,TResult? Function( ServerCommandOutcome_Feeding value)?  feeding,TResult? Function( ServerCommandOutcome_Fed value)?  fed,TResult? Function( ServerCommandOutcome_Unrecorded value)?  unrecorded,}){
final _that = this;
switch (_that) {
case ServerCommandOutcome_Done() when done != null:
return done(_that);case ServerCommandOutcome_Settings() when settings != null:
return settings(_that);case ServerCommandOutcome_Plans() when plans != null:
return plans(_that);case ServerCommandOutcome_Calibration() when calibration != null:
return calibration(_that);case ServerCommandOutcome_Call() when call != null:
return call(_that);case ServerCommandOutcome_Superseded() when superseded != null:
return superseded(_that);case ServerCommandOutcome_Refused() when refused != null:
return refused(_that);case ServerCommandOutcome_Invited() when invited != null:
return invited(_that);case ServerCommandOutcome_Feeding() when feeding != null:
return feeding(_that);case ServerCommandOutcome_Fed() when fed != null:
return fed(_that);case ServerCommandOutcome_Unrecorded() when unrecorded != null:
return unrecorded(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  done,TResult Function()?  settings,TResult Function()?  plans,TResult Function()?  calibration,TResult Function()?  call,TResult Function()?  superseded,TResult Function( ServerRefusal refusal)?  refused,TResult Function( int invite,  int seconds)?  invited,TResult Function( int feed)?  feeding,TResult Function( int feed,  int requested,  int completed,  FeedOutcome outcome,  FeedReason reason)?  fed,TResult Function()?  unrecorded,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ServerCommandOutcome_Done() when done != null:
return done();case ServerCommandOutcome_Settings() when settings != null:
return settings();case ServerCommandOutcome_Plans() when plans != null:
return plans();case ServerCommandOutcome_Calibration() when calibration != null:
return calibration();case ServerCommandOutcome_Call() when call != null:
return call();case ServerCommandOutcome_Superseded() when superseded != null:
return superseded();case ServerCommandOutcome_Refused() when refused != null:
return refused(_that.refusal);case ServerCommandOutcome_Invited() when invited != null:
return invited(_that.invite,_that.seconds);case ServerCommandOutcome_Feeding() when feeding != null:
return feeding(_that.feed);case ServerCommandOutcome_Fed() when fed != null:
return fed(_that.feed,_that.requested,_that.completed,_that.outcome,_that.reason);case ServerCommandOutcome_Unrecorded() when unrecorded != null:
return unrecorded();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  done,required TResult Function()  settings,required TResult Function()  plans,required TResult Function()  calibration,required TResult Function()  call,required TResult Function()  superseded,required TResult Function( ServerRefusal refusal)  refused,required TResult Function( int invite,  int seconds)  invited,required TResult Function( int feed)  feeding,required TResult Function( int feed,  int requested,  int completed,  FeedOutcome outcome,  FeedReason reason)  fed,required TResult Function()  unrecorded,}) {final _that = this;
switch (_that) {
case ServerCommandOutcome_Done():
return done();case ServerCommandOutcome_Settings():
return settings();case ServerCommandOutcome_Plans():
return plans();case ServerCommandOutcome_Calibration():
return calibration();case ServerCommandOutcome_Call():
return call();case ServerCommandOutcome_Superseded():
return superseded();case ServerCommandOutcome_Refused():
return refused(_that.refusal);case ServerCommandOutcome_Invited():
return invited(_that.invite,_that.seconds);case ServerCommandOutcome_Feeding():
return feeding(_that.feed);case ServerCommandOutcome_Fed():
return fed(_that.feed,_that.requested,_that.completed,_that.outcome,_that.reason);case ServerCommandOutcome_Unrecorded():
return unrecorded();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  done,TResult? Function()?  settings,TResult? Function()?  plans,TResult? Function()?  calibration,TResult? Function()?  call,TResult? Function()?  superseded,TResult? Function( ServerRefusal refusal)?  refused,TResult? Function( int invite,  int seconds)?  invited,TResult? Function( int feed)?  feeding,TResult? Function( int feed,  int requested,  int completed,  FeedOutcome outcome,  FeedReason reason)?  fed,TResult? Function()?  unrecorded,}) {final _that = this;
switch (_that) {
case ServerCommandOutcome_Done() when done != null:
return done();case ServerCommandOutcome_Settings() when settings != null:
return settings();case ServerCommandOutcome_Plans() when plans != null:
return plans();case ServerCommandOutcome_Calibration() when calibration != null:
return calibration();case ServerCommandOutcome_Call() when call != null:
return call();case ServerCommandOutcome_Superseded() when superseded != null:
return superseded();case ServerCommandOutcome_Refused() when refused != null:
return refused(_that.refusal);case ServerCommandOutcome_Invited() when invited != null:
return invited(_that.invite,_that.seconds);case ServerCommandOutcome_Feeding() when feeding != null:
return feeding(_that.feed);case ServerCommandOutcome_Fed() when fed != null:
return fed(_that.feed,_that.requested,_that.completed,_that.outcome,_that.reason);case ServerCommandOutcome_Unrecorded() when unrecorded != null:
return unrecorded();case _:
  return null;

}
}

}

/// @nodoc


class ServerCommandOutcome_Done extends ServerCommandOutcome {
  const ServerCommandOutcome_Done(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Done);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerCommandOutcome.done()';
}


}




/// @nodoc


class ServerCommandOutcome_Settings extends ServerCommandOutcome {
  const ServerCommandOutcome_Settings(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Settings);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerCommandOutcome.settings()';
}


}




/// @nodoc


class ServerCommandOutcome_Plans extends ServerCommandOutcome {
  const ServerCommandOutcome_Plans(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Plans);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerCommandOutcome.plans()';
}


}




/// @nodoc


class ServerCommandOutcome_Calibration extends ServerCommandOutcome {
  const ServerCommandOutcome_Calibration(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Calibration);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerCommandOutcome.calibration()';
}


}




/// @nodoc


class ServerCommandOutcome_Call extends ServerCommandOutcome {
  const ServerCommandOutcome_Call(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Call);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerCommandOutcome.call()';
}


}




/// @nodoc


class ServerCommandOutcome_Superseded extends ServerCommandOutcome {
  const ServerCommandOutcome_Superseded(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Superseded);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerCommandOutcome.superseded()';
}


}




/// @nodoc


class ServerCommandOutcome_Refused extends ServerCommandOutcome {
  const ServerCommandOutcome_Refused({required this.refusal}): super._();
  

 final  ServerRefusal refusal;

/// Create a copy of ServerCommandOutcome
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerCommandOutcome_RefusedCopyWith<ServerCommandOutcome_Refused> get copyWith => _$ServerCommandOutcome_RefusedCopyWithImpl<ServerCommandOutcome_Refused>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Refused&&(identical(other.refusal, refusal) || other.refusal == refusal));
}


@override
int get hashCode {
    return Object.hash(runtimeType,refusal);
}

@override
String toString() {
    return 'ServerCommandOutcome.refused(refusal: $refusal)';
}


}

/// @nodoc
abstract mixin class $ServerCommandOutcome_RefusedCopyWith<$Res> implements $ServerCommandOutcomeCopyWith<$Res> {
  factory $ServerCommandOutcome_RefusedCopyWith(ServerCommandOutcome_Refused value, $Res Function(ServerCommandOutcome_Refused) _then) = _$ServerCommandOutcome_RefusedCopyWithImpl;
@useResult
$Res call({
 ServerRefusal refusal
});




}
/// @nodoc
class _$ServerCommandOutcome_RefusedCopyWithImpl<$Res>
    implements $ServerCommandOutcome_RefusedCopyWith<$Res> {
  _$ServerCommandOutcome_RefusedCopyWithImpl(this._self, this._then);

  final ServerCommandOutcome_Refused _self;
  final $Res Function(ServerCommandOutcome_Refused) _then;

/// Create a copy of ServerCommandOutcome
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? refusal = null,}) {
  return _then(ServerCommandOutcome_Refused(
refusal: null == refusal ? _self.refusal : refusal // ignore: cast_nullable_to_non_nullable
as ServerRefusal,
  ));
}


}

/// @nodoc


class ServerCommandOutcome_Invited extends ServerCommandOutcome {
  const ServerCommandOutcome_Invited({required this.invite, required this.seconds}): super._();
  

 final  int invite;
 final  int seconds;

/// Create a copy of ServerCommandOutcome
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerCommandOutcome_InvitedCopyWith<ServerCommandOutcome_Invited> get copyWith => _$ServerCommandOutcome_InvitedCopyWithImpl<ServerCommandOutcome_Invited>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Invited&&(identical(other.invite, invite) || other.invite == invite)&&(identical(other.seconds, seconds) || other.seconds == seconds));
}


@override
int get hashCode {
    return Object.hash(runtimeType,invite,seconds);
}

@override
String toString() {
    return 'ServerCommandOutcome.invited(invite: $invite, seconds: $seconds)';
}


}

/// @nodoc
abstract mixin class $ServerCommandOutcome_InvitedCopyWith<$Res> implements $ServerCommandOutcomeCopyWith<$Res> {
  factory $ServerCommandOutcome_InvitedCopyWith(ServerCommandOutcome_Invited value, $Res Function(ServerCommandOutcome_Invited) _then) = _$ServerCommandOutcome_InvitedCopyWithImpl;
@useResult
$Res call({
 int invite, int seconds
});




}
/// @nodoc
class _$ServerCommandOutcome_InvitedCopyWithImpl<$Res>
    implements $ServerCommandOutcome_InvitedCopyWith<$Res> {
  _$ServerCommandOutcome_InvitedCopyWithImpl(this._self, this._then);

  final ServerCommandOutcome_Invited _self;
  final $Res Function(ServerCommandOutcome_Invited) _then;

/// Create a copy of ServerCommandOutcome
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? invite = null,Object? seconds = null,}) {
  return _then(ServerCommandOutcome_Invited(
invite: null == invite ? _self.invite : invite // ignore: cast_nullable_to_non_nullable
as int,seconds: null == seconds ? _self.seconds : seconds // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class ServerCommandOutcome_Feeding extends ServerCommandOutcome {
  const ServerCommandOutcome_Feeding({required this.feed}): super._();
  

 final  int feed;

/// Create a copy of ServerCommandOutcome
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerCommandOutcome_FeedingCopyWith<ServerCommandOutcome_Feeding> get copyWith => _$ServerCommandOutcome_FeedingCopyWithImpl<ServerCommandOutcome_Feeding>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Feeding&&(identical(other.feed, feed) || other.feed == feed));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feed);
}

@override
String toString() {
    return 'ServerCommandOutcome.feeding(feed: $feed)';
}


}

/// @nodoc
abstract mixin class $ServerCommandOutcome_FeedingCopyWith<$Res> implements $ServerCommandOutcomeCopyWith<$Res> {
  factory $ServerCommandOutcome_FeedingCopyWith(ServerCommandOutcome_Feeding value, $Res Function(ServerCommandOutcome_Feeding) _then) = _$ServerCommandOutcome_FeedingCopyWithImpl;
@useResult
$Res call({
 int feed
});




}
/// @nodoc
class _$ServerCommandOutcome_FeedingCopyWithImpl<$Res>
    implements $ServerCommandOutcome_FeedingCopyWith<$Res> {
  _$ServerCommandOutcome_FeedingCopyWithImpl(this._self, this._then);

  final ServerCommandOutcome_Feeding _self;
  final $Res Function(ServerCommandOutcome_Feeding) _then;

/// Create a copy of ServerCommandOutcome
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? feed = null,}) {
  return _then(ServerCommandOutcome_Feeding(
feed: null == feed ? _self.feed : feed // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class ServerCommandOutcome_Fed extends ServerCommandOutcome {
  const ServerCommandOutcome_Fed({required this.feed, required this.requested, required this.completed, required this.outcome, required this.reason}): super._();
  

 final  int feed;
 final  int requested;
 final  int completed;
 final  FeedOutcome outcome;
 final  FeedReason reason;

/// Create a copy of ServerCommandOutcome
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerCommandOutcome_FedCopyWith<ServerCommandOutcome_Fed> get copyWith => _$ServerCommandOutcome_FedCopyWithImpl<ServerCommandOutcome_Fed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Fed&&(identical(other.feed, feed) || other.feed == feed)&&(identical(other.requested, requested) || other.requested == requested)&&(identical(other.completed, completed) || other.completed == completed)&&(identical(other.outcome, outcome) || other.outcome == outcome)&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feed,requested,completed,outcome,reason);
}

@override
String toString() {
    return 'ServerCommandOutcome.fed(feed: $feed, requested: $requested, completed: $completed, outcome: $outcome, reason: $reason)';
}


}

/// @nodoc
abstract mixin class $ServerCommandOutcome_FedCopyWith<$Res> implements $ServerCommandOutcomeCopyWith<$Res> {
  factory $ServerCommandOutcome_FedCopyWith(ServerCommandOutcome_Fed value, $Res Function(ServerCommandOutcome_Fed) _then) = _$ServerCommandOutcome_FedCopyWithImpl;
@useResult
$Res call({
 int feed, int requested, int completed, FeedOutcome outcome, FeedReason reason
});




}
/// @nodoc
class _$ServerCommandOutcome_FedCopyWithImpl<$Res>
    implements $ServerCommandOutcome_FedCopyWith<$Res> {
  _$ServerCommandOutcome_FedCopyWithImpl(this._self, this._then);

  final ServerCommandOutcome_Fed _self;
  final $Res Function(ServerCommandOutcome_Fed) _then;

/// Create a copy of ServerCommandOutcome
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? feed = null,Object? requested = null,Object? completed = null,Object? outcome = null,Object? reason = null,}) {
  return _then(ServerCommandOutcome_Fed(
feed: null == feed ? _self.feed : feed // ignore: cast_nullable_to_non_nullable
as int,requested: null == requested ? _self.requested : requested // ignore: cast_nullable_to_non_nullable
as int,completed: null == completed ? _self.completed : completed // ignore: cast_nullable_to_non_nullable
as int,outcome: null == outcome ? _self.outcome : outcome // ignore: cast_nullable_to_non_nullable
as FeedOutcome,reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as FeedReason,
  ));
}


}

/// @nodoc


class ServerCommandOutcome_Unrecorded extends ServerCommandOutcome {
  const ServerCommandOutcome_Unrecorded(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerCommandOutcome_Unrecorded);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ServerCommandOutcome.unrecorded()';
}


}




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
