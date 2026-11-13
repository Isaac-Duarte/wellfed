// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'bluetooth.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$ScanEvent {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ScanEvent);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'ScanEvent()';
}


}

/// @nodoc
class $ScanEventCopyWith<$Res>  {
$ScanEventCopyWith(ScanEvent _, $Res Function(ScanEvent) __);
}


/// Adds pattern-matching-related methods to [ScanEvent].
extension ScanEventPatterns on ScanEvent {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ScanEvent_Heard value)?  heard,TResult Function( ScanEvent_Failed value)?  failed,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ScanEvent_Heard() when heard != null:
return heard(_that);case ScanEvent_Failed() when failed != null:
return failed(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ScanEvent_Heard value)  heard,required TResult Function( ScanEvent_Failed value)  failed,}){
final _that = this;
switch (_that) {
case ScanEvent_Heard():
return heard(_that);case ScanEvent_Failed():
return failed(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ScanEvent_Heard value)?  heard,TResult? Function( ScanEvent_Failed value)?  failed,}){
final _that = this;
switch (_that) {
case ScanEvent_Heard() when heard != null:
return heard(_that);case ScanEvent_Failed() when failed != null:
return failed(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( HeardFeeder feeder)?  heard,TResult Function( BridgeError error)?  failed,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ScanEvent_Heard() when heard != null:
return heard(_that.feeder);case ScanEvent_Failed() when failed != null:
return failed(_that.error);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( HeardFeeder feeder)  heard,required TResult Function( BridgeError error)  failed,}) {final _that = this;
switch (_that) {
case ScanEvent_Heard():
return heard(_that.feeder);case ScanEvent_Failed():
return failed(_that.error);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( HeardFeeder feeder)?  heard,TResult? Function( BridgeError error)?  failed,}) {final _that = this;
switch (_that) {
case ScanEvent_Heard() when heard != null:
return heard(_that.feeder);case ScanEvent_Failed() when failed != null:
return failed(_that.error);case _:
  return null;

}
}

}

/// @nodoc


class ScanEvent_Heard extends ScanEvent {
  const ScanEvent_Heard({required this.feeder}): super._();
  

 final  HeardFeeder feeder;

/// Create a copy of ScanEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ScanEvent_HeardCopyWith<ScanEvent_Heard> get copyWith => _$ScanEvent_HeardCopyWithImpl<ScanEvent_Heard>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ScanEvent_Heard&&(identical(other.feeder, feeder) || other.feeder == feeder));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feeder);
}

@override
String toString() {
    return 'ScanEvent.heard(feeder: $feeder)';
}


}

/// @nodoc
abstract mixin class $ScanEvent_HeardCopyWith<$Res> implements $ScanEventCopyWith<$Res> {
  factory $ScanEvent_HeardCopyWith(ScanEvent_Heard value, $Res Function(ScanEvent_Heard) _then) = _$ScanEvent_HeardCopyWithImpl;
@useResult
$Res call({
 HeardFeeder feeder
});




}
/// @nodoc
class _$ScanEvent_HeardCopyWithImpl<$Res>
    implements $ScanEvent_HeardCopyWith<$Res> {
  _$ScanEvent_HeardCopyWithImpl(this._self, this._then);

  final ScanEvent_Heard _self;
  final $Res Function(ScanEvent_Heard) _then;

/// Create a copy of ScanEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? feeder = null,}) {
  return _then(ScanEvent_Heard(
feeder: null == feeder ? _self.feeder : feeder // ignore: cast_nullable_to_non_nullable
as HeardFeeder,
  ));
}


}

/// @nodoc


class ScanEvent_Failed extends ScanEvent {
  const ScanEvent_Failed({required this.error}): super._();
  

 final  BridgeError error;

/// Create a copy of ScanEvent
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ScanEvent_FailedCopyWith<ScanEvent_Failed> get copyWith => _$ScanEvent_FailedCopyWithImpl<ScanEvent_Failed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ScanEvent_Failed&&(identical(other.error, error) || other.error == error));
}


@override
int get hashCode {
    return Object.hash(runtimeType,error);
}

@override
String toString() {
    return 'ScanEvent.failed(error: $error)';
}


}

/// @nodoc
abstract mixin class $ScanEvent_FailedCopyWith<$Res> implements $ScanEventCopyWith<$Res> {
  factory $ScanEvent_FailedCopyWith(ScanEvent_Failed value, $Res Function(ScanEvent_Failed) _then) = _$ScanEvent_FailedCopyWithImpl;
@useResult
$Res call({
 BridgeError error
});




}
/// @nodoc
class _$ScanEvent_FailedCopyWithImpl<$Res>
    implements $ScanEvent_FailedCopyWith<$Res> {
  _$ScanEvent_FailedCopyWithImpl(this._self, this._then);

  final ScanEvent_Failed _self;
  final $Res Function(ScanEvent_Failed) _then;

/// Create a copy of ScanEvent
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? error = null,}) {
  return _then(ScanEvent_Failed(
error: null == error ? _self.error : error // ignore: cast_nullable_to_non_nullable
as BridgeError,
  ));
}


}

// dart format on
