// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'router.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$RetryOutcome {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RetryOutcome);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'RetryOutcome()';
}


}

/// @nodoc
class $RetryOutcomeCopyWith<$Res>  {
$RetryOutcomeCopyWith(RetryOutcome _, $Res Function(RetryOutcome) __);
}


/// Adds pattern-matching-related methods to [RetryOutcome].
extension RetryOutcomePatterns on RetryOutcome {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( RetryOutcome_Settled value)?  settled,TResult Function( RetryOutcome_Uncertain value)?  uncertain,TResult Function( RetryOutcome_Unknown value)?  unknown,required TResult orElse(),}){
final _that = this;
switch (_that) {
case RetryOutcome_Settled() when settled != null:
return settled(_that);case RetryOutcome_Uncertain() when uncertain != null:
return uncertain(_that);case RetryOutcome_Unknown() when unknown != null:
return unknown(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( RetryOutcome_Settled value)  settled,required TResult Function( RetryOutcome_Uncertain value)  uncertain,required TResult Function( RetryOutcome_Unknown value)  unknown,}){
final _that = this;
switch (_that) {
case RetryOutcome_Settled():
return settled(_that);case RetryOutcome_Uncertain():
return uncertain(_that);case RetryOutcome_Unknown():
return unknown(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( RetryOutcome_Settled value)?  settled,TResult? Function( RetryOutcome_Uncertain value)?  uncertain,TResult? Function( RetryOutcome_Unknown value)?  unknown,}){
final _that = this;
switch (_that) {
case RetryOutcome_Settled() when settled != null:
return settled(_that);case RetryOutcome_Uncertain() when uncertain != null:
return uncertain(_that);case RetryOutcome_Unknown() when unknown != null:
return unknown(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( PendingWrite pending,  Settlement settlement)?  settled,TResult Function( PendingWrite pending)?  uncertain,TResult Function()?  unknown,required TResult orElse(),}) {final _that = this;
switch (_that) {
case RetryOutcome_Settled() when settled != null:
return settled(_that.pending,_that.settlement);case RetryOutcome_Uncertain() when uncertain != null:
return uncertain(_that.pending);case RetryOutcome_Unknown() when unknown != null:
return unknown();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( PendingWrite pending,  Settlement settlement)  settled,required TResult Function( PendingWrite pending)  uncertain,required TResult Function()  unknown,}) {final _that = this;
switch (_that) {
case RetryOutcome_Settled():
return settled(_that.pending,_that.settlement);case RetryOutcome_Uncertain():
return uncertain(_that.pending);case RetryOutcome_Unknown():
return unknown();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( PendingWrite pending,  Settlement settlement)?  settled,TResult? Function( PendingWrite pending)?  uncertain,TResult? Function()?  unknown,}) {final _that = this;
switch (_that) {
case RetryOutcome_Settled() when settled != null:
return settled(_that.pending,_that.settlement);case RetryOutcome_Uncertain() when uncertain != null:
return uncertain(_that.pending);case RetryOutcome_Unknown() when unknown != null:
return unknown();case _:
  return null;

}
}

}

/// @nodoc


class RetryOutcome_Settled extends RetryOutcome {
  const RetryOutcome_Settled({required this.pending, required this.settlement}): super._();
  

 final  PendingWrite pending;
 final  Settlement settlement;

/// Create a copy of RetryOutcome
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$RetryOutcome_SettledCopyWith<RetryOutcome_Settled> get copyWith => _$RetryOutcome_SettledCopyWithImpl<RetryOutcome_Settled>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RetryOutcome_Settled&&(identical(other.pending, pending) || other.pending == pending)&&(identical(other.settlement, settlement) || other.settlement == settlement));
}


@override
int get hashCode {
    return Object.hash(runtimeType,pending,settlement);
}

@override
String toString() {
    return 'RetryOutcome.settled(pending: $pending, settlement: $settlement)';
}


}

/// @nodoc
abstract mixin class $RetryOutcome_SettledCopyWith<$Res> implements $RetryOutcomeCopyWith<$Res> {
  factory $RetryOutcome_SettledCopyWith(RetryOutcome_Settled value, $Res Function(RetryOutcome_Settled) _then) = _$RetryOutcome_SettledCopyWithImpl;
@useResult
$Res call({
 PendingWrite pending, Settlement settlement
});


$SettlementCopyWith<$Res> get settlement;

}
/// @nodoc
class _$RetryOutcome_SettledCopyWithImpl<$Res>
    implements $RetryOutcome_SettledCopyWith<$Res> {
  _$RetryOutcome_SettledCopyWithImpl(this._self, this._then);

  final RetryOutcome_Settled _self;
  final $Res Function(RetryOutcome_Settled) _then;

/// Create a copy of RetryOutcome
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? pending = null,Object? settlement = null,}) {
  return _then(RetryOutcome_Settled(
pending: null == pending ? _self.pending : pending // ignore: cast_nullable_to_non_nullable
as PendingWrite,settlement: null == settlement ? _self.settlement : settlement // ignore: cast_nullable_to_non_nullable
as Settlement,
  ));
}

/// Create a copy of RetryOutcome
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$SettlementCopyWith<$Res> get settlement {
  
  return $SettlementCopyWith<$Res>(_self.settlement, (value) {
    return _then(_self.copyWith(settlement: value));
  });
}
}

/// @nodoc


class RetryOutcome_Uncertain extends RetryOutcome {
  const RetryOutcome_Uncertain({required this.pending}): super._();
  

 final  PendingWrite pending;

/// Create a copy of RetryOutcome
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$RetryOutcome_UncertainCopyWith<RetryOutcome_Uncertain> get copyWith => _$RetryOutcome_UncertainCopyWithImpl<RetryOutcome_Uncertain>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RetryOutcome_Uncertain&&(identical(other.pending, pending) || other.pending == pending));
}


@override
int get hashCode {
    return Object.hash(runtimeType,pending);
}

@override
String toString() {
    return 'RetryOutcome.uncertain(pending: $pending)';
}


}

/// @nodoc
abstract mixin class $RetryOutcome_UncertainCopyWith<$Res> implements $RetryOutcomeCopyWith<$Res> {
  factory $RetryOutcome_UncertainCopyWith(RetryOutcome_Uncertain value, $Res Function(RetryOutcome_Uncertain) _then) = _$RetryOutcome_UncertainCopyWithImpl;
@useResult
$Res call({
 PendingWrite pending
});




}
/// @nodoc
class _$RetryOutcome_UncertainCopyWithImpl<$Res>
    implements $RetryOutcome_UncertainCopyWith<$Res> {
  _$RetryOutcome_UncertainCopyWithImpl(this._self, this._then);

  final RetryOutcome_Uncertain _self;
  final $Res Function(RetryOutcome_Uncertain) _then;

/// Create a copy of RetryOutcome
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? pending = null,}) {
  return _then(RetryOutcome_Uncertain(
pending: null == pending ? _self.pending : pending // ignore: cast_nullable_to_non_nullable
as PendingWrite,
  ));
}


}

/// @nodoc


class RetryOutcome_Unknown extends RetryOutcome {
  const RetryOutcome_Unknown(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RetryOutcome_Unknown);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'RetryOutcome.unknown()';
}


}




/// @nodoc
mixin _$RoutedFeed {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RoutedFeed);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'RoutedFeed()';
}


}

/// @nodoc
class $RoutedFeedCopyWith<$Res>  {
$RoutedFeedCopyWith(RoutedFeed _, $Res Function(RoutedFeed) __);
}


/// Adds pattern-matching-related methods to [RoutedFeed].
extension RoutedFeedPatterns on RoutedFeed {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( RoutedFeed_Started value)?  started,TResult Function( RoutedFeed_Refused value)?  refused,TResult Function( RoutedFeed_Uncertain value)?  uncertain,TResult Function( RoutedFeed_NotSent value)?  notSent,TResult Function( RoutedFeed_Held value)?  held,required TResult orElse(),}){
final _that = this;
switch (_that) {
case RoutedFeed_Started() when started != null:
return started(_that);case RoutedFeed_Refused() when refused != null:
return refused(_that);case RoutedFeed_Uncertain() when uncertain != null:
return uncertain(_that);case RoutedFeed_NotSent() when notSent != null:
return notSent(_that);case RoutedFeed_Held() when held != null:
return held(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( RoutedFeed_Started value)  started,required TResult Function( RoutedFeed_Refused value)  refused,required TResult Function( RoutedFeed_Uncertain value)  uncertain,required TResult Function( RoutedFeed_NotSent value)  notSent,required TResult Function( RoutedFeed_Held value)  held,}){
final _that = this;
switch (_that) {
case RoutedFeed_Started():
return started(_that);case RoutedFeed_Refused():
return refused(_that);case RoutedFeed_Uncertain():
return uncertain(_that);case RoutedFeed_NotSent():
return notSent(_that);case RoutedFeed_Held():
return held(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( RoutedFeed_Started value)?  started,TResult? Function( RoutedFeed_Refused value)?  refused,TResult? Function( RoutedFeed_Uncertain value)?  uncertain,TResult? Function( RoutedFeed_NotSent value)?  notSent,TResult? Function( RoutedFeed_Held value)?  held,}){
final _that = this;
switch (_that) {
case RoutedFeed_Started() when started != null:
return started(_that);case RoutedFeed_Refused() when refused != null:
return refused(_that);case RoutedFeed_Uncertain() when uncertain != null:
return uncertain(_that);case RoutedFeed_NotSent() when notSent != null:
return notSent(_that);case RoutedFeed_Held() when held != null:
return held(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String key,  int feed,  int portions,  int dueSeconds)?  started,TResult Function( String key,  FeedRefusal refusal)?  refused,TResult Function( PendingWrite pending)?  uncertain,TResult Function( NotSentReason reason)?  notSent,TResult Function( PendingWrite pending)?  held,required TResult orElse(),}) {final _that = this;
switch (_that) {
case RoutedFeed_Started() when started != null:
return started(_that.key,_that.feed,_that.portions,_that.dueSeconds);case RoutedFeed_Refused() when refused != null:
return refused(_that.key,_that.refusal);case RoutedFeed_Uncertain() when uncertain != null:
return uncertain(_that.pending);case RoutedFeed_NotSent() when notSent != null:
return notSent(_that.reason);case RoutedFeed_Held() when held != null:
return held(_that.pending);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String key,  int feed,  int portions,  int dueSeconds)  started,required TResult Function( String key,  FeedRefusal refusal)  refused,required TResult Function( PendingWrite pending)  uncertain,required TResult Function( NotSentReason reason)  notSent,required TResult Function( PendingWrite pending)  held,}) {final _that = this;
switch (_that) {
case RoutedFeed_Started():
return started(_that.key,_that.feed,_that.portions,_that.dueSeconds);case RoutedFeed_Refused():
return refused(_that.key,_that.refusal);case RoutedFeed_Uncertain():
return uncertain(_that.pending);case RoutedFeed_NotSent():
return notSent(_that.reason);case RoutedFeed_Held():
return held(_that.pending);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String key,  int feed,  int portions,  int dueSeconds)?  started,TResult? Function( String key,  FeedRefusal refusal)?  refused,TResult? Function( PendingWrite pending)?  uncertain,TResult? Function( NotSentReason reason)?  notSent,TResult? Function( PendingWrite pending)?  held,}) {final _that = this;
switch (_that) {
case RoutedFeed_Started() when started != null:
return started(_that.key,_that.feed,_that.portions,_that.dueSeconds);case RoutedFeed_Refused() when refused != null:
return refused(_that.key,_that.refusal);case RoutedFeed_Uncertain() when uncertain != null:
return uncertain(_that.pending);case RoutedFeed_NotSent() when notSent != null:
return notSent(_that.reason);case RoutedFeed_Held() when held != null:
return held(_that.pending);case _:
  return null;

}
}

}

/// @nodoc


class RoutedFeed_Started extends RoutedFeed {
  const RoutedFeed_Started({required this.key, required this.feed, required this.portions, required this.dueSeconds}): super._();
  

 final  String key;
 final  int feed;
 final  int portions;
/// Past this a feeder that stayed on has ended it itself.
 final  int dueSeconds;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$RoutedFeed_StartedCopyWith<RoutedFeed_Started> get copyWith => _$RoutedFeed_StartedCopyWithImpl<RoutedFeed_Started>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RoutedFeed_Started&&(identical(other.key, key) || other.key == key)&&(identical(other.feed, feed) || other.feed == feed)&&(identical(other.portions, portions) || other.portions == portions)&&(identical(other.dueSeconds, dueSeconds) || other.dueSeconds == dueSeconds));
}


@override
int get hashCode {
    return Object.hash(runtimeType,key,feed,portions,dueSeconds);
}

@override
String toString() {
    return 'RoutedFeed.started(key: $key, feed: $feed, portions: $portions, dueSeconds: $dueSeconds)';
}


}

/// @nodoc
abstract mixin class $RoutedFeed_StartedCopyWith<$Res> implements $RoutedFeedCopyWith<$Res> {
  factory $RoutedFeed_StartedCopyWith(RoutedFeed_Started value, $Res Function(RoutedFeed_Started) _then) = _$RoutedFeed_StartedCopyWithImpl;
@useResult
$Res call({
 String key, int feed, int portions, int dueSeconds
});




}
/// @nodoc
class _$RoutedFeed_StartedCopyWithImpl<$Res>
    implements $RoutedFeed_StartedCopyWith<$Res> {
  _$RoutedFeed_StartedCopyWithImpl(this._self, this._then);

  final RoutedFeed_Started _self;
  final $Res Function(RoutedFeed_Started) _then;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? key = null,Object? feed = null,Object? portions = null,Object? dueSeconds = null,}) {
  return _then(RoutedFeed_Started(
key: null == key ? _self.key : key // ignore: cast_nullable_to_non_nullable
as String,feed: null == feed ? _self.feed : feed // ignore: cast_nullable_to_non_nullable
as int,portions: null == portions ? _self.portions : portions // ignore: cast_nullable_to_non_nullable
as int,dueSeconds: null == dueSeconds ? _self.dueSeconds : dueSeconds // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class RoutedFeed_Refused extends RoutedFeed {
  const RoutedFeed_Refused({required this.key, required this.refusal}): super._();
  

 final  String key;
 final  FeedRefusal refusal;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$RoutedFeed_RefusedCopyWith<RoutedFeed_Refused> get copyWith => _$RoutedFeed_RefusedCopyWithImpl<RoutedFeed_Refused>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RoutedFeed_Refused&&(identical(other.key, key) || other.key == key)&&(identical(other.refusal, refusal) || other.refusal == refusal));
}


@override
int get hashCode {
    return Object.hash(runtimeType,key,refusal);
}

@override
String toString() {
    return 'RoutedFeed.refused(key: $key, refusal: $refusal)';
}


}

/// @nodoc
abstract mixin class $RoutedFeed_RefusedCopyWith<$Res> implements $RoutedFeedCopyWith<$Res> {
  factory $RoutedFeed_RefusedCopyWith(RoutedFeed_Refused value, $Res Function(RoutedFeed_Refused) _then) = _$RoutedFeed_RefusedCopyWithImpl;
@useResult
$Res call({
 String key, FeedRefusal refusal
});




}
/// @nodoc
class _$RoutedFeed_RefusedCopyWithImpl<$Res>
    implements $RoutedFeed_RefusedCopyWith<$Res> {
  _$RoutedFeed_RefusedCopyWithImpl(this._self, this._then);

  final RoutedFeed_Refused _self;
  final $Res Function(RoutedFeed_Refused) _then;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? key = null,Object? refusal = null,}) {
  return _then(RoutedFeed_Refused(
key: null == key ? _self.key : key // ignore: cast_nullable_to_non_nullable
as String,refusal: null == refusal ? _self.refusal : refusal // ignore: cast_nullable_to_non_nullable
as FeedRefusal,
  ));
}


}

/// @nodoc


class RoutedFeed_Uncertain extends RoutedFeed {
  const RoutedFeed_Uncertain({required this.pending}): super._();
  

 final  PendingWrite pending;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$RoutedFeed_UncertainCopyWith<RoutedFeed_Uncertain> get copyWith => _$RoutedFeed_UncertainCopyWithImpl<RoutedFeed_Uncertain>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RoutedFeed_Uncertain&&(identical(other.pending, pending) || other.pending == pending));
}


@override
int get hashCode {
    return Object.hash(runtimeType,pending);
}

@override
String toString() {
    return 'RoutedFeed.uncertain(pending: $pending)';
}


}

/// @nodoc
abstract mixin class $RoutedFeed_UncertainCopyWith<$Res> implements $RoutedFeedCopyWith<$Res> {
  factory $RoutedFeed_UncertainCopyWith(RoutedFeed_Uncertain value, $Res Function(RoutedFeed_Uncertain) _then) = _$RoutedFeed_UncertainCopyWithImpl;
@useResult
$Res call({
 PendingWrite pending
});




}
/// @nodoc
class _$RoutedFeed_UncertainCopyWithImpl<$Res>
    implements $RoutedFeed_UncertainCopyWith<$Res> {
  _$RoutedFeed_UncertainCopyWithImpl(this._self, this._then);

  final RoutedFeed_Uncertain _self;
  final $Res Function(RoutedFeed_Uncertain) _then;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? pending = null,}) {
  return _then(RoutedFeed_Uncertain(
pending: null == pending ? _self.pending : pending // ignore: cast_nullable_to_non_nullable
as PendingWrite,
  ));
}


}

/// @nodoc


class RoutedFeed_NotSent extends RoutedFeed {
  const RoutedFeed_NotSent({required this.reason}): super._();
  

 final  NotSentReason reason;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$RoutedFeed_NotSentCopyWith<RoutedFeed_NotSent> get copyWith => _$RoutedFeed_NotSentCopyWithImpl<RoutedFeed_NotSent>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RoutedFeed_NotSent&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode {
    return Object.hash(runtimeType,reason);
}

@override
String toString() {
    return 'RoutedFeed.notSent(reason: $reason)';
}


}

/// @nodoc
abstract mixin class $RoutedFeed_NotSentCopyWith<$Res> implements $RoutedFeedCopyWith<$Res> {
  factory $RoutedFeed_NotSentCopyWith(RoutedFeed_NotSent value, $Res Function(RoutedFeed_NotSent) _then) = _$RoutedFeed_NotSentCopyWithImpl;
@useResult
$Res call({
 NotSentReason reason
});




}
/// @nodoc
class _$RoutedFeed_NotSentCopyWithImpl<$Res>
    implements $RoutedFeed_NotSentCopyWith<$Res> {
  _$RoutedFeed_NotSentCopyWithImpl(this._self, this._then);

  final RoutedFeed_NotSent _self;
  final $Res Function(RoutedFeed_NotSent) _then;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? reason = null,}) {
  return _then(RoutedFeed_NotSent(
reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as NotSentReason,
  ));
}


}

/// @nodoc


class RoutedFeed_Held extends RoutedFeed {
  const RoutedFeed_Held({required this.pending}): super._();
  

 final  PendingWrite pending;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$RoutedFeed_HeldCopyWith<RoutedFeed_Held> get copyWith => _$RoutedFeed_HeldCopyWithImpl<RoutedFeed_Held>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is RoutedFeed_Held&&(identical(other.pending, pending) || other.pending == pending));
}


@override
int get hashCode {
    return Object.hash(runtimeType,pending);
}

@override
String toString() {
    return 'RoutedFeed.held(pending: $pending)';
}


}

/// @nodoc
abstract mixin class $RoutedFeed_HeldCopyWith<$Res> implements $RoutedFeedCopyWith<$Res> {
  factory $RoutedFeed_HeldCopyWith(RoutedFeed_Held value, $Res Function(RoutedFeed_Held) _then) = _$RoutedFeed_HeldCopyWithImpl;
@useResult
$Res call({
 PendingWrite pending
});




}
/// @nodoc
class _$RoutedFeed_HeldCopyWithImpl<$Res>
    implements $RoutedFeed_HeldCopyWith<$Res> {
  _$RoutedFeed_HeldCopyWithImpl(this._self, this._then);

  final RoutedFeed_Held _self;
  final $Res Function(RoutedFeed_Held) _then;

/// Create a copy of RoutedFeed
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? pending = null,}) {
  return _then(RoutedFeed_Held(
pending: null == pending ? _self.pending : pending // ignore: cast_nullable_to_non_nullable
as PendingWrite,
  ));
}


}

/// @nodoc
mixin _$Settlement {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Settlement);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Settlement()';
}


}

/// @nodoc
class $SettlementCopyWith<$Res>  {
$SettlementCopyWith(Settlement _, $Res Function(Settlement) __);
}


/// Adds pattern-matching-related methods to [Settlement].
extension SettlementPatterns on Settlement {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( Settlement_Refused value)?  refused,TResult Function( Settlement_Started value)?  started,TResult Function( Settlement_Fed value)?  fed,TResult Function( Settlement_NotSent value)?  notSent,required TResult orElse(),}){
final _that = this;
switch (_that) {
case Settlement_Refused() when refused != null:
return refused(_that);case Settlement_Started() when started != null:
return started(_that);case Settlement_Fed() when fed != null:
return fed(_that);case Settlement_NotSent() when notSent != null:
return notSent(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( Settlement_Refused value)  refused,required TResult Function( Settlement_Started value)  started,required TResult Function( Settlement_Fed value)  fed,required TResult Function( Settlement_NotSent value)  notSent,}){
final _that = this;
switch (_that) {
case Settlement_Refused():
return refused(_that);case Settlement_Started():
return started(_that);case Settlement_Fed():
return fed(_that);case Settlement_NotSent():
return notSent(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( Settlement_Refused value)?  refused,TResult? Function( Settlement_Started value)?  started,TResult? Function( Settlement_Fed value)?  fed,TResult? Function( Settlement_NotSent value)?  notSent,}){
final _that = this;
switch (_that) {
case Settlement_Refused() when refused != null:
return refused(_that);case Settlement_Started() when started != null:
return started(_that);case Settlement_Fed() when fed != null:
return fed(_that);case Settlement_NotSent() when notSent != null:
return notSent(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( FeedRefusal refusal)?  refused,TResult Function( int feed)?  started,TResult Function( int feed,  int requested,  int completed,  FeedOutcome outcome,  FeedReason reason)?  fed,TResult Function()?  notSent,required TResult orElse(),}) {final _that = this;
switch (_that) {
case Settlement_Refused() when refused != null:
return refused(_that.refusal);case Settlement_Started() when started != null:
return started(_that.feed);case Settlement_Fed() when fed != null:
return fed(_that.feed,_that.requested,_that.completed,_that.outcome,_that.reason);case Settlement_NotSent() when notSent != null:
return notSent();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( FeedRefusal refusal)  refused,required TResult Function( int feed)  started,required TResult Function( int feed,  int requested,  int completed,  FeedOutcome outcome,  FeedReason reason)  fed,required TResult Function()  notSent,}) {final _that = this;
switch (_that) {
case Settlement_Refused():
return refused(_that.refusal);case Settlement_Started():
return started(_that.feed);case Settlement_Fed():
return fed(_that.feed,_that.requested,_that.completed,_that.outcome,_that.reason);case Settlement_NotSent():
return notSent();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( FeedRefusal refusal)?  refused,TResult? Function( int feed)?  started,TResult? Function( int feed,  int requested,  int completed,  FeedOutcome outcome,  FeedReason reason)?  fed,TResult? Function()?  notSent,}) {final _that = this;
switch (_that) {
case Settlement_Refused() when refused != null:
return refused(_that.refusal);case Settlement_Started() when started != null:
return started(_that.feed);case Settlement_Fed() when fed != null:
return fed(_that.feed,_that.requested,_that.completed,_that.outcome,_that.reason);case Settlement_NotSent() when notSent != null:
return notSent();case _:
  return null;

}
}

}

/// @nodoc


class Settlement_Refused extends Settlement {
  const Settlement_Refused({required this.refusal}): super._();
  

 final  FeedRefusal refusal;

/// Create a copy of Settlement
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Settlement_RefusedCopyWith<Settlement_Refused> get copyWith => _$Settlement_RefusedCopyWithImpl<Settlement_Refused>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Settlement_Refused&&(identical(other.refusal, refusal) || other.refusal == refusal));
}


@override
int get hashCode {
    return Object.hash(runtimeType,refusal);
}

@override
String toString() {
    return 'Settlement.refused(refusal: $refusal)';
}


}

/// @nodoc
abstract mixin class $Settlement_RefusedCopyWith<$Res> implements $SettlementCopyWith<$Res> {
  factory $Settlement_RefusedCopyWith(Settlement_Refused value, $Res Function(Settlement_Refused) _then) = _$Settlement_RefusedCopyWithImpl;
@useResult
$Res call({
 FeedRefusal refusal
});




}
/// @nodoc
class _$Settlement_RefusedCopyWithImpl<$Res>
    implements $Settlement_RefusedCopyWith<$Res> {
  _$Settlement_RefusedCopyWithImpl(this._self, this._then);

  final Settlement_Refused _self;
  final $Res Function(Settlement_Refused) _then;

/// Create a copy of Settlement
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? refusal = null,}) {
  return _then(Settlement_Refused(
refusal: null == refusal ? _self.refusal : refusal // ignore: cast_nullable_to_non_nullable
as FeedRefusal,
  ));
}


}

/// @nodoc


class Settlement_Started extends Settlement {
  const Settlement_Started({required this.feed}): super._();
  

 final  int feed;

/// Create a copy of Settlement
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Settlement_StartedCopyWith<Settlement_Started> get copyWith => _$Settlement_StartedCopyWithImpl<Settlement_Started>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Settlement_Started&&(identical(other.feed, feed) || other.feed == feed));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feed);
}

@override
String toString() {
    return 'Settlement.started(feed: $feed)';
}


}

/// @nodoc
abstract mixin class $Settlement_StartedCopyWith<$Res> implements $SettlementCopyWith<$Res> {
  factory $Settlement_StartedCopyWith(Settlement_Started value, $Res Function(Settlement_Started) _then) = _$Settlement_StartedCopyWithImpl;
@useResult
$Res call({
 int feed
});




}
/// @nodoc
class _$Settlement_StartedCopyWithImpl<$Res>
    implements $Settlement_StartedCopyWith<$Res> {
  _$Settlement_StartedCopyWithImpl(this._self, this._then);

  final Settlement_Started _self;
  final $Res Function(Settlement_Started) _then;

/// Create a copy of Settlement
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? feed = null,}) {
  return _then(Settlement_Started(
feed: null == feed ? _self.feed : feed // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class Settlement_Fed extends Settlement {
  const Settlement_Fed({required this.feed, required this.requested, required this.completed, required this.outcome, required this.reason}): super._();
  

 final  int feed;
 final  int requested;
 final  int completed;
 final  FeedOutcome outcome;
 final  FeedReason reason;

/// Create a copy of Settlement
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Settlement_FedCopyWith<Settlement_Fed> get copyWith => _$Settlement_FedCopyWithImpl<Settlement_Fed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Settlement_Fed&&(identical(other.feed, feed) || other.feed == feed)&&(identical(other.requested, requested) || other.requested == requested)&&(identical(other.completed, completed) || other.completed == completed)&&(identical(other.outcome, outcome) || other.outcome == outcome)&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feed,requested,completed,outcome,reason);
}

@override
String toString() {
    return 'Settlement.fed(feed: $feed, requested: $requested, completed: $completed, outcome: $outcome, reason: $reason)';
}


}

/// @nodoc
abstract mixin class $Settlement_FedCopyWith<$Res> implements $SettlementCopyWith<$Res> {
  factory $Settlement_FedCopyWith(Settlement_Fed value, $Res Function(Settlement_Fed) _then) = _$Settlement_FedCopyWithImpl;
@useResult
$Res call({
 int feed, int requested, int completed, FeedOutcome outcome, FeedReason reason
});




}
/// @nodoc
class _$Settlement_FedCopyWithImpl<$Res>
    implements $Settlement_FedCopyWith<$Res> {
  _$Settlement_FedCopyWithImpl(this._self, this._then);

  final Settlement_Fed _self;
  final $Res Function(Settlement_Fed) _then;

/// Create a copy of Settlement
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? feed = null,Object? requested = null,Object? completed = null,Object? outcome = null,Object? reason = null,}) {
  return _then(Settlement_Fed(
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


class Settlement_NotSent extends Settlement {
  const Settlement_NotSent(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Settlement_NotSent);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Settlement.notSent()';
}


}




// dart format on
