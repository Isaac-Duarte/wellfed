// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'server_views.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$NewRefill {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NewRefill);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'NewRefill()';
}


}

/// @nodoc
class $NewRefillCopyWith<$Res>  {
$NewRefillCopyWith(NewRefill _, $Res Function(NewRefill) __);
}


/// Adds pattern-matching-related methods to [NewRefill].
extension NewRefillPatterns on NewRefill {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( NewRefill_Grams value)?  grams,TResult Function( NewRefill_FullBag value)?  fullBag,required TResult orElse(),}){
final _that = this;
switch (_that) {
case NewRefill_Grams() when grams != null:
return grams(_that);case NewRefill_FullBag() when fullBag != null:
return fullBag(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( NewRefill_Grams value)  grams,required TResult Function( NewRefill_FullBag value)  fullBag,}){
final _that = this;
switch (_that) {
case NewRefill_Grams():
return grams(_that);case NewRefill_FullBag():
return fullBag(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( NewRefill_Grams value)?  grams,TResult? Function( NewRefill_FullBag value)?  fullBag,}){
final _that = this;
switch (_that) {
case NewRefill_Grams() when grams != null:
return grams(_that);case NewRefill_FullBag() when fullBag != null:
return fullBag(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( int grams)?  grams,TResult Function()?  fullBag,required TResult orElse(),}) {final _that = this;
switch (_that) {
case NewRefill_Grams() when grams != null:
return grams(_that.grams);case NewRefill_FullBag() when fullBag != null:
return fullBag();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( int grams)  grams,required TResult Function()  fullBag,}) {final _that = this;
switch (_that) {
case NewRefill_Grams():
return grams(_that.grams);case NewRefill_FullBag():
return fullBag();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( int grams)?  grams,TResult? Function()?  fullBag,}) {final _that = this;
switch (_that) {
case NewRefill_Grams() when grams != null:
return grams(_that.grams);case NewRefill_FullBag() when fullBag != null:
return fullBag();case _:
  return null;

}
}

}

/// @nodoc


class NewRefill_Grams extends NewRefill {
  const NewRefill_Grams({required this.grams}): super._();
  

 final  int grams;

/// Create a copy of NewRefill
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NewRefill_GramsCopyWith<NewRefill_Grams> get copyWith => _$NewRefill_GramsCopyWithImpl<NewRefill_Grams>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NewRefill_Grams&&(identical(other.grams, grams) || other.grams == grams));
}


@override
int get hashCode {
    return Object.hash(runtimeType,grams);
}

@override
String toString() {
    return 'NewRefill.grams(grams: $grams)';
}


}

/// @nodoc
abstract mixin class $NewRefill_GramsCopyWith<$Res> implements $NewRefillCopyWith<$Res> {
  factory $NewRefill_GramsCopyWith(NewRefill_Grams value, $Res Function(NewRefill_Grams) _then) = _$NewRefill_GramsCopyWithImpl;
@useResult
$Res call({
 int grams
});




}
/// @nodoc
class _$NewRefill_GramsCopyWithImpl<$Res>
    implements $NewRefill_GramsCopyWith<$Res> {
  _$NewRefill_GramsCopyWithImpl(this._self, this._then);

  final NewRefill_Grams _self;
  final $Res Function(NewRefill_Grams) _then;

/// Create a copy of NewRefill
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? grams = null,}) {
  return _then(NewRefill_Grams(
grams: null == grams ? _self.grams : grams // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class NewRefill_FullBag extends NewRefill {
  const NewRefill_FullBag(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NewRefill_FullBag);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'NewRefill.fullBag()';
}


}




/// @nodoc
mixin _$PortionWeight {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is PortionWeight);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'PortionWeight()';
}


}

/// @nodoc
class $PortionWeightCopyWith<$Res>  {
$PortionWeightCopyWith(PortionWeight _, $Res Function(PortionWeight) __);
}


/// Adds pattern-matching-related methods to [PortionWeight].
extension PortionWeightPatterns on PortionWeight {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( PortionWeight_Calibration value)?  calibration,TResult Function( PortionWeight_Uncalibrated value)?  uncalibrated,required TResult orElse(),}){
final _that = this;
switch (_that) {
case PortionWeight_Calibration() when calibration != null:
return calibration(_that);case PortionWeight_Uncalibrated() when uncalibrated != null:
return uncalibrated(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( PortionWeight_Calibration value)  calibration,required TResult Function( PortionWeight_Uncalibrated value)  uncalibrated,}){
final _that = this;
switch (_that) {
case PortionWeight_Calibration():
return calibration(_that);case PortionWeight_Uncalibrated():
return uncalibrated(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( PortionWeight_Calibration value)?  calibration,TResult? Function( PortionWeight_Uncalibrated value)?  uncalibrated,}){
final _that = this;
switch (_that) {
case PortionWeight_Calibration() when calibration != null:
return calibration(_that);case PortionWeight_Uncalibrated() when uncalibrated != null:
return uncalibrated(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( int portionMg)?  calibration,TResult Function()?  uncalibrated,required TResult orElse(),}) {final _that = this;
switch (_that) {
case PortionWeight_Calibration() when calibration != null:
return calibration(_that.portionMg);case PortionWeight_Uncalibrated() when uncalibrated != null:
return uncalibrated();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( int portionMg)  calibration,required TResult Function()  uncalibrated,}) {final _that = this;
switch (_that) {
case PortionWeight_Calibration():
return calibration(_that.portionMg);case PortionWeight_Uncalibrated():
return uncalibrated();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( int portionMg)?  calibration,TResult? Function()?  uncalibrated,}) {final _that = this;
switch (_that) {
case PortionWeight_Calibration() when calibration != null:
return calibration(_that.portionMg);case PortionWeight_Uncalibrated() when uncalibrated != null:
return uncalibrated();case _:
  return null;

}
}

}

/// @nodoc


class PortionWeight_Calibration extends PortionWeight {
  const PortionWeight_Calibration({required this.portionMg}): super._();
  

 final  int portionMg;

/// Create a copy of PortionWeight
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$PortionWeight_CalibrationCopyWith<PortionWeight_Calibration> get copyWith => _$PortionWeight_CalibrationCopyWithImpl<PortionWeight_Calibration>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is PortionWeight_Calibration&&(identical(other.portionMg, portionMg) || other.portionMg == portionMg));
}


@override
int get hashCode {
    return Object.hash(runtimeType,portionMg);
}

@override
String toString() {
    return 'PortionWeight.calibration(portionMg: $portionMg)';
}


}

/// @nodoc
abstract mixin class $PortionWeight_CalibrationCopyWith<$Res> implements $PortionWeightCopyWith<$Res> {
  factory $PortionWeight_CalibrationCopyWith(PortionWeight_Calibration value, $Res Function(PortionWeight_Calibration) _then) = _$PortionWeight_CalibrationCopyWithImpl;
@useResult
$Res call({
 int portionMg
});




}
/// @nodoc
class _$PortionWeight_CalibrationCopyWithImpl<$Res>
    implements $PortionWeight_CalibrationCopyWith<$Res> {
  _$PortionWeight_CalibrationCopyWithImpl(this._self, this._then);

  final PortionWeight_Calibration _self;
  final $Res Function(PortionWeight_Calibration) _then;

/// Create a copy of PortionWeight
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? portionMg = null,}) {
  return _then(PortionWeight_Calibration(
portionMg: null == portionMg ? _self.portionMg : portionMg // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class PortionWeight_Uncalibrated extends PortionWeight {
  const PortionWeight_Uncalibrated(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is PortionWeight_Uncalibrated);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'PortionWeight.uncalibrated()';
}


}




/// @nodoc
mixin _$ServerAnomaly {

 String get feeder;
/// Create a copy of ServerAnomaly
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerAnomalyCopyWith<ServerAnomaly> get copyWith => _$ServerAnomalyCopyWithImpl<ServerAnomaly>(this as ServerAnomaly, _$identity);



@override
bool operator ==(Object other) {
  final _this = this as ServerAnomaly;
  return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerAnomaly&&(identical(other.feeder, _this.feeder) || other.feeder == _this.feeder));
}


@override
int get hashCode {
  final _this = this as ServerAnomaly;
  return Object.hash(runtimeType,_this.feeder);
}

@override
String toString() {
  final _this = this as ServerAnomaly;
  return 'ServerAnomaly(feeder: ${_this.feeder})';
}


}

/// @nodoc
abstract mixin class $ServerAnomalyCopyWith<$Res>  {
  factory $ServerAnomalyCopyWith(ServerAnomaly value, $Res Function(ServerAnomaly) _then) = _$ServerAnomalyCopyWithImpl;
@useResult
$Res call({
 String feeder
});




}
/// @nodoc
class _$ServerAnomalyCopyWithImpl<$Res>
    implements $ServerAnomalyCopyWith<$Res> {
  _$ServerAnomalyCopyWithImpl(this._self, this._then);

  final ServerAnomaly _self;
  final $Res Function(ServerAnomaly) _then;

/// Create a copy of ServerAnomaly
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') @override $Res call({Object? feeder = null,}) {
  return _then(_self.copyWith(
feeder: null == feeder ? _self.feeder : feeder // ignore: cast_nullable_to_non_nullable
as String,
  ));
}

}


/// Adds pattern-matching-related methods to [ServerAnomaly].
extension ServerAnomalyPatterns on ServerAnomaly {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( ServerAnomaly_Missed value)?  missed,TResult Function( ServerAnomaly_Uncertain value)?  uncertain,TResult Function( ServerAnomaly_Short value)?  short,required TResult orElse(),}){
final _that = this;
switch (_that) {
case ServerAnomaly_Missed() when missed != null:
return missed(_that);case ServerAnomaly_Uncertain() when uncertain != null:
return uncertain(_that);case ServerAnomaly_Short() when short != null:
return short(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( ServerAnomaly_Missed value)  missed,required TResult Function( ServerAnomaly_Uncertain value)  uncertain,required TResult Function( ServerAnomaly_Short value)  short,}){
final _that = this;
switch (_that) {
case ServerAnomaly_Missed():
return missed(_that);case ServerAnomaly_Uncertain():
return uncertain(_that);case ServerAnomaly_Short():
return short(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( ServerAnomaly_Missed value)?  missed,TResult? Function( ServerAnomaly_Uncertain value)?  uncertain,TResult? Function( ServerAnomaly_Short value)?  short,}){
final _that = this;
switch (_that) {
case ServerAnomaly_Missed() when missed != null:
return missed(_that);case ServerAnomaly_Uncertain() when uncertain != null:
return uncertain(_that);case ServerAnomaly_Short() when short != null:
return short(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String feeder,  int plan,  PlatformInt64 dueMs,  MissedReason reason)?  missed,TResult Function( String feeder,  int feed,  PlatformInt64 atMs,  int requested,  int completed)?  uncertain,TResult Function( String feeder,  int feed,  PlatformInt64 atMs,  int requested,  int completed,  FeedOutcome outcome,  FeedReason reason)?  short,required TResult orElse(),}) {final _that = this;
switch (_that) {
case ServerAnomaly_Missed() when missed != null:
return missed(_that.feeder,_that.plan,_that.dueMs,_that.reason);case ServerAnomaly_Uncertain() when uncertain != null:
return uncertain(_that.feeder,_that.feed,_that.atMs,_that.requested,_that.completed);case ServerAnomaly_Short() when short != null:
return short(_that.feeder,_that.feed,_that.atMs,_that.requested,_that.completed,_that.outcome,_that.reason);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String feeder,  int plan,  PlatformInt64 dueMs,  MissedReason reason)  missed,required TResult Function( String feeder,  int feed,  PlatformInt64 atMs,  int requested,  int completed)  uncertain,required TResult Function( String feeder,  int feed,  PlatformInt64 atMs,  int requested,  int completed,  FeedOutcome outcome,  FeedReason reason)  short,}) {final _that = this;
switch (_that) {
case ServerAnomaly_Missed():
return missed(_that.feeder,_that.plan,_that.dueMs,_that.reason);case ServerAnomaly_Uncertain():
return uncertain(_that.feeder,_that.feed,_that.atMs,_that.requested,_that.completed);case ServerAnomaly_Short():
return short(_that.feeder,_that.feed,_that.atMs,_that.requested,_that.completed,_that.outcome,_that.reason);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String feeder,  int plan,  PlatformInt64 dueMs,  MissedReason reason)?  missed,TResult? Function( String feeder,  int feed,  PlatformInt64 atMs,  int requested,  int completed)?  uncertain,TResult? Function( String feeder,  int feed,  PlatformInt64 atMs,  int requested,  int completed,  FeedOutcome outcome,  FeedReason reason)?  short,}) {final _that = this;
switch (_that) {
case ServerAnomaly_Missed() when missed != null:
return missed(_that.feeder,_that.plan,_that.dueMs,_that.reason);case ServerAnomaly_Uncertain() when uncertain != null:
return uncertain(_that.feeder,_that.feed,_that.atMs,_that.requested,_that.completed);case ServerAnomaly_Short() when short != null:
return short(_that.feeder,_that.feed,_that.atMs,_that.requested,_that.completed,_that.outcome,_that.reason);case _:
  return null;

}
}

}

/// @nodoc


class ServerAnomaly_Missed extends ServerAnomaly {
  const ServerAnomaly_Missed({required this.feeder, required this.plan, required this.dueMs, required this.reason}): super._();
  

@override final  String feeder;
 final  int plan;
 final  PlatformInt64 dueMs;
 final  MissedReason reason;

/// Create a copy of ServerAnomaly
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerAnomaly_MissedCopyWith<ServerAnomaly_Missed> get copyWith => _$ServerAnomaly_MissedCopyWithImpl<ServerAnomaly_Missed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerAnomaly_Missed&&(identical(other.feeder, feeder) || other.feeder == feeder)&&(identical(other.plan, plan) || other.plan == plan)&&(identical(other.dueMs, dueMs) || other.dueMs == dueMs)&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feeder,plan,dueMs,reason);
}

@override
String toString() {
    return 'ServerAnomaly.missed(feeder: $feeder, plan: $plan, dueMs: $dueMs, reason: $reason)';
}


}

/// @nodoc
abstract mixin class $ServerAnomaly_MissedCopyWith<$Res> implements $ServerAnomalyCopyWith<$Res> {
  factory $ServerAnomaly_MissedCopyWith(ServerAnomaly_Missed value, $Res Function(ServerAnomaly_Missed) _then) = _$ServerAnomaly_MissedCopyWithImpl;
@override @useResult
$Res call({
 String feeder, int plan, PlatformInt64 dueMs, MissedReason reason
});


$MissedReasonCopyWith<$Res> get reason;

}
/// @nodoc
class _$ServerAnomaly_MissedCopyWithImpl<$Res>
    implements $ServerAnomaly_MissedCopyWith<$Res> {
  _$ServerAnomaly_MissedCopyWithImpl(this._self, this._then);

  final ServerAnomaly_Missed _self;
  final $Res Function(ServerAnomaly_Missed) _then;

/// Create a copy of ServerAnomaly
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? feeder = null,Object? plan = null,Object? dueMs = null,Object? reason = null,}) {
  return _then(ServerAnomaly_Missed(
feeder: null == feeder ? _self.feeder : feeder // ignore: cast_nullable_to_non_nullable
as String,plan: null == plan ? _self.plan : plan // ignore: cast_nullable_to_non_nullable
as int,dueMs: null == dueMs ? _self.dueMs : dueMs // ignore: cast_nullable_to_non_nullable
as PlatformInt64,reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as MissedReason,
  ));
}

/// Create a copy of ServerAnomaly
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$MissedReasonCopyWith<$Res> get reason {
  
  return $MissedReasonCopyWith<$Res>(_self.reason, (value) {
    return _then(_self.copyWith(reason: value));
  });
}
}

/// @nodoc


class ServerAnomaly_Uncertain extends ServerAnomaly {
  const ServerAnomaly_Uncertain({required this.feeder, required this.feed, required this.atMs, required this.requested, required this.completed}): super._();
  

@override final  String feeder;
 final  int feed;
 final  PlatformInt64 atMs;
 final  int requested;
 final  int completed;

/// Create a copy of ServerAnomaly
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerAnomaly_UncertainCopyWith<ServerAnomaly_Uncertain> get copyWith => _$ServerAnomaly_UncertainCopyWithImpl<ServerAnomaly_Uncertain>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerAnomaly_Uncertain&&(identical(other.feeder, feeder) || other.feeder == feeder)&&(identical(other.feed, feed) || other.feed == feed)&&(identical(other.atMs, atMs) || other.atMs == atMs)&&(identical(other.requested, requested) || other.requested == requested)&&(identical(other.completed, completed) || other.completed == completed));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feeder,feed,atMs,requested,completed);
}

@override
String toString() {
    return 'ServerAnomaly.uncertain(feeder: $feeder, feed: $feed, atMs: $atMs, requested: $requested, completed: $completed)';
}


}

/// @nodoc
abstract mixin class $ServerAnomaly_UncertainCopyWith<$Res> implements $ServerAnomalyCopyWith<$Res> {
  factory $ServerAnomaly_UncertainCopyWith(ServerAnomaly_Uncertain value, $Res Function(ServerAnomaly_Uncertain) _then) = _$ServerAnomaly_UncertainCopyWithImpl;
@override @useResult
$Res call({
 String feeder, int feed, PlatformInt64 atMs, int requested, int completed
});




}
/// @nodoc
class _$ServerAnomaly_UncertainCopyWithImpl<$Res>
    implements $ServerAnomaly_UncertainCopyWith<$Res> {
  _$ServerAnomaly_UncertainCopyWithImpl(this._self, this._then);

  final ServerAnomaly_Uncertain _self;
  final $Res Function(ServerAnomaly_Uncertain) _then;

/// Create a copy of ServerAnomaly
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? feeder = null,Object? feed = null,Object? atMs = null,Object? requested = null,Object? completed = null,}) {
  return _then(ServerAnomaly_Uncertain(
feeder: null == feeder ? _self.feeder : feeder // ignore: cast_nullable_to_non_nullable
as String,feed: null == feed ? _self.feed : feed // ignore: cast_nullable_to_non_nullable
as int,atMs: null == atMs ? _self.atMs : atMs // ignore: cast_nullable_to_non_nullable
as PlatformInt64,requested: null == requested ? _self.requested : requested // ignore: cast_nullable_to_non_nullable
as int,completed: null == completed ? _self.completed : completed // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class ServerAnomaly_Short extends ServerAnomaly {
  const ServerAnomaly_Short({required this.feeder, required this.feed, required this.atMs, required this.requested, required this.completed, required this.outcome, required this.reason}): super._();
  

@override final  String feeder;
 final  int feed;
 final  PlatformInt64 atMs;
 final  int requested;
 final  int completed;
 final  FeedOutcome outcome;
 final  FeedReason reason;

/// Create a copy of ServerAnomaly
/// with the given fields replaced by the non-null parameter values.
@override @JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$ServerAnomaly_ShortCopyWith<ServerAnomaly_Short> get copyWith => _$ServerAnomaly_ShortCopyWithImpl<ServerAnomaly_Short>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is ServerAnomaly_Short&&(identical(other.feeder, feeder) || other.feeder == feeder)&&(identical(other.feed, feed) || other.feed == feed)&&(identical(other.atMs, atMs) || other.atMs == atMs)&&(identical(other.requested, requested) || other.requested == requested)&&(identical(other.completed, completed) || other.completed == completed)&&(identical(other.outcome, outcome) || other.outcome == outcome)&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feeder,feed,atMs,requested,completed,outcome,reason);
}

@override
String toString() {
    return 'ServerAnomaly.short(feeder: $feeder, feed: $feed, atMs: $atMs, requested: $requested, completed: $completed, outcome: $outcome, reason: $reason)';
}


}

/// @nodoc
abstract mixin class $ServerAnomaly_ShortCopyWith<$Res> implements $ServerAnomalyCopyWith<$Res> {
  factory $ServerAnomaly_ShortCopyWith(ServerAnomaly_Short value, $Res Function(ServerAnomaly_Short) _then) = _$ServerAnomaly_ShortCopyWithImpl;
@override @useResult
$Res call({
 String feeder, int feed, PlatformInt64 atMs, int requested, int completed, FeedOutcome outcome, FeedReason reason
});




}
/// @nodoc
class _$ServerAnomaly_ShortCopyWithImpl<$Res>
    implements $ServerAnomaly_ShortCopyWith<$Res> {
  _$ServerAnomaly_ShortCopyWithImpl(this._self, this._then);

  final ServerAnomaly_Short _self;
  final $Res Function(ServerAnomaly_Short) _then;

/// Create a copy of ServerAnomaly
/// with the given fields replaced by the non-null parameter values.
@override @pragma('vm:prefer-inline') $Res call({Object? feeder = null,Object? feed = null,Object? atMs = null,Object? requested = null,Object? completed = null,Object? outcome = null,Object? reason = null,}) {
  return _then(ServerAnomaly_Short(
feeder: null == feeder ? _self.feeder : feeder // ignore: cast_nullable_to_non_nullable
as String,feed: null == feed ? _self.feed : feed // ignore: cast_nullable_to_non_nullable
as int,atMs: null == atMs ? _self.atMs : atMs // ignore: cast_nullable_to_non_nullable
as PlatformInt64,requested: null == requested ? _self.requested : requested // ignore: cast_nullable_to_non_nullable
as int,completed: null == completed ? _self.completed : completed // ignore: cast_nullable_to_non_nullable
as int,outcome: null == outcome ? _self.outcome : outcome // ignore: cast_nullable_to_non_nullable
as FeedOutcome,reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as FeedReason,
  ));
}


}

// dart format on
