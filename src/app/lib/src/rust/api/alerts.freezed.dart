// GENERATED CODE - DO NOT MODIFY BY HAND
// coverage:ignore-file
// ignore_for_file: type=lint, type=warning, deprecated_member_use, deprecated_member_use_from_same_package
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'alerts.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

// GENERATED CODE - DO NOT MODIFY BY HAND
// dart format off
T _$identity<T>(T value) => value;
/// @nodoc
mixin _$Attempt {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Attempt);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Attempt()';
}


}

/// @nodoc
class $AttemptCopyWith<$Res>  {
$AttemptCopyWith(Attempt _, $Res Function(Attempt) __);
}


/// Adds pattern-matching-related methods to [Attempt].
extension AttemptPatterns on Attempt {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( Attempt_First value)?  first,TResult Function( Attempt_Last value)?  last,required TResult orElse(),}){
final _that = this;
switch (_that) {
case Attempt_First() when first != null:
return first(_that);case Attempt_Last() when last != null:
return last(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( Attempt_First value)  first,required TResult Function( Attempt_Last value)  last,}){
final _that = this;
switch (_that) {
case Attempt_First():
return first(_that);case Attempt_Last():
return last(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( Attempt_First value)?  first,TResult? Function( Attempt_Last value)?  last,}){
final _that = this;
switch (_that) {
case Attempt_First() when first != null:
return first(_that);case Attempt_Last() when last != null:
return last(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function()?  first,TResult Function( String? located)?  last,required TResult orElse(),}) {final _that = this;
switch (_that) {
case Attempt_First() when first != null:
return first();case Attempt_Last() when last != null:
return last(_that.located);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function()  first,required TResult Function( String? located)  last,}) {final _that = this;
switch (_that) {
case Attempt_First():
return first();case Attempt_Last():
return last(_that.located);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function()?  first,TResult? Function( String? located)?  last,}) {final _that = this;
switch (_that) {
case Attempt_First() when first != null:
return first();case Attempt_Last() when last != null:
return last(_that.located);case _:
  return null;

}
}

}

/// @nodoc


class Attempt_First extends Attempt {
  const Attempt_First(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Attempt_First);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'Attempt.first()';
}


}




/// @nodoc


class Attempt_Last extends Attempt {
  const Attempt_Last({this.located}): super._();
  

 final  String? located;

/// Create a copy of Attempt
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$Attempt_LastCopyWith<Attempt_Last> get copyWith => _$Attempt_LastCopyWithImpl<Attempt_Last>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is Attempt_Last&&(identical(other.located, located) || other.located == located));
}


@override
int get hashCode {
    return Object.hash(runtimeType,located);
}

@override
String toString() {
    return 'Attempt.last(located: $located)';
}


}

/// @nodoc
abstract mixin class $Attempt_LastCopyWith<$Res> implements $AttemptCopyWith<$Res> {
  factory $Attempt_LastCopyWith(Attempt_Last value, $Res Function(Attempt_Last) _then) = _$Attempt_LastCopyWithImpl;
@useResult
$Res call({
 String? located
});




}
/// @nodoc
class _$Attempt_LastCopyWithImpl<$Res>
    implements $Attempt_LastCopyWith<$Res> {
  _$Attempt_LastCopyWithImpl(this._self, this._then);

  final Attempt_Last _self;
  final $Res Function(Attempt_Last) _then;

/// Create a copy of Attempt
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? located = freezed,}) {
  return _then(Attempt_Last(
located: freezed == located ? _self.located : located // ignore: cast_nullable_to_non_nullable
as String?,
  ));
}


}

/// @nodoc
mixin _$MuteTarget {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is MuteTarget);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'MuteTarget()';
}


}

/// @nodoc
class $MuteTargetCopyWith<$Res>  {
$MuteTargetCopyWith(MuteTarget _, $Res Function(MuteTarget) __);
}


/// Adds pattern-matching-related methods to [MuteTarget].
extension MuteTargetPatterns on MuteTarget {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( MuteTarget_Kind value)?  kind,TResult Function( MuteTarget_Level value)?  level,required TResult orElse(),}){
final _that = this;
switch (_that) {
case MuteTarget_Kind() when kind != null:
return kind(_that);case MuteTarget_Level() when level != null:
return level(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( MuteTarget_Kind value)  kind,required TResult Function( MuteTarget_Level value)  level,}){
final _that = this;
switch (_that) {
case MuteTarget_Kind():
return kind(_that);case MuteTarget_Level():
return level(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( MuteTarget_Kind value)?  kind,TResult? Function( MuteTarget_Level value)?  level,}){
final _that = this;
switch (_that) {
case MuteTarget_Kind() when kind != null:
return kind(_that);case MuteTarget_Level() when level != null:
return level(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( NoticeKind kind)?  kind,TResult Function( AlertLevel level)?  level,required TResult orElse(),}) {final _that = this;
switch (_that) {
case MuteTarget_Kind() when kind != null:
return kind(_that.kind);case MuteTarget_Level() when level != null:
return level(_that.level);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( NoticeKind kind)  kind,required TResult Function( AlertLevel level)  level,}) {final _that = this;
switch (_that) {
case MuteTarget_Kind():
return kind(_that.kind);case MuteTarget_Level():
return level(_that.level);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( NoticeKind kind)?  kind,TResult? Function( AlertLevel level)?  level,}) {final _that = this;
switch (_that) {
case MuteTarget_Kind() when kind != null:
return kind(_that.kind);case MuteTarget_Level() when level != null:
return level(_that.level);case _:
  return null;

}
}

}

/// @nodoc


class MuteTarget_Kind extends MuteTarget {
  const MuteTarget_Kind({required this.kind}): super._();
  

 final  NoticeKind kind;

/// Create a copy of MuteTarget
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MuteTarget_KindCopyWith<MuteTarget_Kind> get copyWith => _$MuteTarget_KindCopyWithImpl<MuteTarget_Kind>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is MuteTarget_Kind&&(identical(other.kind, kind) || other.kind == kind));
}


@override
int get hashCode {
    return Object.hash(runtimeType,kind);
}

@override
String toString() {
    return 'MuteTarget.kind(kind: $kind)';
}


}

/// @nodoc
abstract mixin class $MuteTarget_KindCopyWith<$Res> implements $MuteTargetCopyWith<$Res> {
  factory $MuteTarget_KindCopyWith(MuteTarget_Kind value, $Res Function(MuteTarget_Kind) _then) = _$MuteTarget_KindCopyWithImpl;
@useResult
$Res call({
 NoticeKind kind
});




}
/// @nodoc
class _$MuteTarget_KindCopyWithImpl<$Res>
    implements $MuteTarget_KindCopyWith<$Res> {
  _$MuteTarget_KindCopyWithImpl(this._self, this._then);

  final MuteTarget_Kind _self;
  final $Res Function(MuteTarget_Kind) _then;

/// Create a copy of MuteTarget
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? kind = null,}) {
  return _then(MuteTarget_Kind(
kind: null == kind ? _self.kind : kind // ignore: cast_nullable_to_non_nullable
as NoticeKind,
  ));
}


}

/// @nodoc


class MuteTarget_Level extends MuteTarget {
  const MuteTarget_Level({required this.level}): super._();
  

 final  AlertLevel level;

/// Create a copy of MuteTarget
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$MuteTarget_LevelCopyWith<MuteTarget_Level> get copyWith => _$MuteTarget_LevelCopyWithImpl<MuteTarget_Level>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is MuteTarget_Level&&(identical(other.level, level) || other.level == level));
}


@override
int get hashCode {
    return Object.hash(runtimeType,level);
}

@override
String toString() {
    return 'MuteTarget.level(level: $level)';
}


}

/// @nodoc
abstract mixin class $MuteTarget_LevelCopyWith<$Res> implements $MuteTargetCopyWith<$Res> {
  factory $MuteTarget_LevelCopyWith(MuteTarget_Level value, $Res Function(MuteTarget_Level) _then) = _$MuteTarget_LevelCopyWithImpl;
@useResult
$Res call({
 AlertLevel level
});




}
/// @nodoc
class _$MuteTarget_LevelCopyWithImpl<$Res>
    implements $MuteTarget_LevelCopyWith<$Res> {
  _$MuteTarget_LevelCopyWithImpl(this._self, this._then);

  final MuteTarget_Level _self;
  final $Res Function(MuteTarget_Level) _then;

/// Create a copy of MuteTarget
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? level = null,}) {
  return _then(MuteTarget_Level(
level: null == level ? _self.level : level // ignore: cast_nullable_to_non_nullable
as AlertLevel,
  ));
}


}

/// @nodoc
mixin _$NoticeChange {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeChange);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'NoticeChange()';
}


}

/// @nodoc
class $NoticeChangeCopyWith<$Res>  {
$NoticeChangeCopyWith(NoticeChange _, $Res Function(NoticeChange) __);
}


/// Adds pattern-matching-related methods to [NoticeChange].
extension NoticeChangePatterns on NoticeChange {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( NoticeChange_Post value)?  post,TResult Function( NoticeChange_Remove value)?  remove,required TResult orElse(),}){
final _that = this;
switch (_that) {
case NoticeChange_Post() when post != null:
return post(_that);case NoticeChange_Remove() when remove != null:
return remove(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( NoticeChange_Post value)  post,required TResult Function( NoticeChange_Remove value)  remove,}){
final _that = this;
switch (_that) {
case NoticeChange_Post():
return post(_that);case NoticeChange_Remove():
return remove(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( NoticeChange_Post value)?  post,TResult? Function( NoticeChange_Remove value)?  remove,}){
final _that = this;
switch (_that) {
case NoticeChange_Post() when post != null:
return post(_that);case NoticeChange_Remove() when remove != null:
return remove(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( NoticeView notice)?  post,TResult Function( int id)?  remove,required TResult orElse(),}) {final _that = this;
switch (_that) {
case NoticeChange_Post() when post != null:
return post(_that.notice);case NoticeChange_Remove() when remove != null:
return remove(_that.id);case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( NoticeView notice)  post,required TResult Function( int id)  remove,}) {final _that = this;
switch (_that) {
case NoticeChange_Post():
return post(_that.notice);case NoticeChange_Remove():
return remove(_that.id);}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( NoticeView notice)?  post,TResult? Function( int id)?  remove,}) {final _that = this;
switch (_that) {
case NoticeChange_Post() when post != null:
return post(_that.notice);case NoticeChange_Remove() when remove != null:
return remove(_that.id);case _:
  return null;

}
}

}

/// @nodoc


class NoticeChange_Post extends NoticeChange {
  const NoticeChange_Post({required this.notice}): super._();
  

 final  NoticeView notice;

/// Create a copy of NoticeChange
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeChange_PostCopyWith<NoticeChange_Post> get copyWith => _$NoticeChange_PostCopyWithImpl<NoticeChange_Post>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeChange_Post&&(identical(other.notice, notice) || other.notice == notice));
}


@override
int get hashCode {
    return Object.hash(runtimeType,notice);
}

@override
String toString() {
    return 'NoticeChange.post(notice: $notice)';
}


}

/// @nodoc
abstract mixin class $NoticeChange_PostCopyWith<$Res> implements $NoticeChangeCopyWith<$Res> {
  factory $NoticeChange_PostCopyWith(NoticeChange_Post value, $Res Function(NoticeChange_Post) _then) = _$NoticeChange_PostCopyWithImpl;
@useResult
$Res call({
 NoticeView notice
});




}
/// @nodoc
class _$NoticeChange_PostCopyWithImpl<$Res>
    implements $NoticeChange_PostCopyWith<$Res> {
  _$NoticeChange_PostCopyWithImpl(this._self, this._then);

  final NoticeChange_Post _self;
  final $Res Function(NoticeChange_Post) _then;

/// Create a copy of NoticeChange
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? notice = null,}) {
  return _then(NoticeChange_Post(
notice: null == notice ? _self.notice : notice // ignore: cast_nullable_to_non_nullable
as NoticeView,
  ));
}


}

/// @nodoc


class NoticeChange_Remove extends NoticeChange {
  const NoticeChange_Remove({required this.id}): super._();
  

 final  int id;

/// Create a copy of NoticeChange
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeChange_RemoveCopyWith<NoticeChange_Remove> get copyWith => _$NoticeChange_RemoveCopyWithImpl<NoticeChange_Remove>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeChange_Remove&&(identical(other.id, id) || other.id == id));
}


@override
int get hashCode {
    return Object.hash(runtimeType,id);
}

@override
String toString() {
    return 'NoticeChange.remove(id: $id)';
}


}

/// @nodoc
abstract mixin class $NoticeChange_RemoveCopyWith<$Res> implements $NoticeChangeCopyWith<$Res> {
  factory $NoticeChange_RemoveCopyWith(NoticeChange_Remove value, $Res Function(NoticeChange_Remove) _then) = _$NoticeChange_RemoveCopyWithImpl;
@useResult
$Res call({
 int id
});




}
/// @nodoc
class _$NoticeChange_RemoveCopyWithImpl<$Res>
    implements $NoticeChange_RemoveCopyWith<$Res> {
  _$NoticeChange_RemoveCopyWithImpl(this._self, this._then);

  final NoticeChange_Remove _self;
  final $Res Function(NoticeChange_Remove) _then;

/// Create a copy of NoticeChange
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? id = null,}) {
  return _then(NoticeChange_Remove(
id: null == id ? _self.id : id // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc
mixin _$NoticeDetail {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'NoticeDetail()';
}


}

/// @nodoc
class $NoticeDetailCopyWith<$Res>  {
$NoticeDetailCopyWith(NoticeDetail _, $Res Function(NoticeDetail) __);
}


/// Adds pattern-matching-related methods to [NoticeDetail].
extension NoticeDetailPatterns on NoticeDetail {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( NoticeDetail_Feed value)?  feed,TResult Function( NoticeDetail_Meal value)?  meal,TResult Function( NoticeDetail_Client value)?  client,TResult Function( NoticeDetail_UpdateFailed value)?  updateFailed,TResult Function( NoticeDetail_CannotFeed value)?  cannotFeed,TResult Function( NoticeDetail_FoodLikelyOut value)?  foodLikelyOut,TResult Function( NoticeDetail_DesiccantDue value)?  desiccantDue,TResult Function( NoticeDetail_Unreachable value)?  unreachable,TResult Function( NoticeDetail_Plain value)?  plain,required TResult orElse(),}){
final _that = this;
switch (_that) {
case NoticeDetail_Feed() when feed != null:
return feed(_that);case NoticeDetail_Meal() when meal != null:
return meal(_that);case NoticeDetail_Client() when client != null:
return client(_that);case NoticeDetail_UpdateFailed() when updateFailed != null:
return updateFailed(_that);case NoticeDetail_CannotFeed() when cannotFeed != null:
return cannotFeed(_that);case NoticeDetail_FoodLikelyOut() when foodLikelyOut != null:
return foodLikelyOut(_that);case NoticeDetail_DesiccantDue() when desiccantDue != null:
return desiccantDue(_that);case NoticeDetail_Unreachable() when unreachable != null:
return unreachable(_that);case NoticeDetail_Plain() when plain != null:
return plain(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( NoticeDetail_Feed value)  feed,required TResult Function( NoticeDetail_Meal value)  meal,required TResult Function( NoticeDetail_Client value)  client,required TResult Function( NoticeDetail_UpdateFailed value)  updateFailed,required TResult Function( NoticeDetail_CannotFeed value)  cannotFeed,required TResult Function( NoticeDetail_FoodLikelyOut value)  foodLikelyOut,required TResult Function( NoticeDetail_DesiccantDue value)  desiccantDue,required TResult Function( NoticeDetail_Unreachable value)  unreachable,required TResult Function( NoticeDetail_Plain value)  plain,}){
final _that = this;
switch (_that) {
case NoticeDetail_Feed():
return feed(_that);case NoticeDetail_Meal():
return meal(_that);case NoticeDetail_Client():
return client(_that);case NoticeDetail_UpdateFailed():
return updateFailed(_that);case NoticeDetail_CannotFeed():
return cannotFeed(_that);case NoticeDetail_FoodLikelyOut():
return foodLikelyOut(_that);case NoticeDetail_DesiccantDue():
return desiccantDue(_that);case NoticeDetail_Unreachable():
return unreachable(_that);case NoticeDetail_Plain():
return plain(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( NoticeDetail_Feed value)?  feed,TResult? Function( NoticeDetail_Meal value)?  meal,TResult? Function( NoticeDetail_Client value)?  client,TResult? Function( NoticeDetail_UpdateFailed value)?  updateFailed,TResult? Function( NoticeDetail_CannotFeed value)?  cannotFeed,TResult? Function( NoticeDetail_FoodLikelyOut value)?  foodLikelyOut,TResult? Function( NoticeDetail_DesiccantDue value)?  desiccantDue,TResult? Function( NoticeDetail_Unreachable value)?  unreachable,TResult? Function( NoticeDetail_Plain value)?  plain,}){
final _that = this;
switch (_that) {
case NoticeDetail_Feed() when feed != null:
return feed(_that);case NoticeDetail_Meal() when meal != null:
return meal(_that);case NoticeDetail_Client() when client != null:
return client(_that);case NoticeDetail_UpdateFailed() when updateFailed != null:
return updateFailed(_that);case NoticeDetail_CannotFeed() when cannotFeed != null:
return cannotFeed(_that);case NoticeDetail_FoodLikelyOut() when foodLikelyOut != null:
return foodLikelyOut(_that);case NoticeDetail_DesiccantDue() when desiccantDue != null:
return desiccantDue(_that);case NoticeDetail_Unreachable() when unreachable != null:
return unreachable(_that);case NoticeDetail_Plain() when plain != null:
return plain(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( int feed,  int? plan,  int requested,  int completed,  FeedOutcome? outcome,  FeedReason reason)?  feed,TResult Function( int plan,  PlatformInt64 dueMs,  MissedReason? missed)?  meal,TResult Function( String client,  ClientRole? role)?  client,TResult Function( UpdateStop stop)?  updateFailed,TResult Function( Blocked reason)?  cannotFeed,TResult Function( int confirmed)?  foodLikelyOut,TResult Function( int days)?  desiccantDue,TResult Function( PlatformInt64 sinceMs)?  unreachable,TResult Function()?  plain,required TResult orElse(),}) {final _that = this;
switch (_that) {
case NoticeDetail_Feed() when feed != null:
return feed(_that.feed,_that.plan,_that.requested,_that.completed,_that.outcome,_that.reason);case NoticeDetail_Meal() when meal != null:
return meal(_that.plan,_that.dueMs,_that.missed);case NoticeDetail_Client() when client != null:
return client(_that.client,_that.role);case NoticeDetail_UpdateFailed() when updateFailed != null:
return updateFailed(_that.stop);case NoticeDetail_CannotFeed() when cannotFeed != null:
return cannotFeed(_that.reason);case NoticeDetail_FoodLikelyOut() when foodLikelyOut != null:
return foodLikelyOut(_that.confirmed);case NoticeDetail_DesiccantDue() when desiccantDue != null:
return desiccantDue(_that.days);case NoticeDetail_Unreachable() when unreachable != null:
return unreachable(_that.sinceMs);case NoticeDetail_Plain() when plain != null:
return plain();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( int feed,  int? plan,  int requested,  int completed,  FeedOutcome? outcome,  FeedReason reason)  feed,required TResult Function( int plan,  PlatformInt64 dueMs,  MissedReason? missed)  meal,required TResult Function( String client,  ClientRole? role)  client,required TResult Function( UpdateStop stop)  updateFailed,required TResult Function( Blocked reason)  cannotFeed,required TResult Function( int confirmed)  foodLikelyOut,required TResult Function( int days)  desiccantDue,required TResult Function( PlatformInt64 sinceMs)  unreachable,required TResult Function()  plain,}) {final _that = this;
switch (_that) {
case NoticeDetail_Feed():
return feed(_that.feed,_that.plan,_that.requested,_that.completed,_that.outcome,_that.reason);case NoticeDetail_Meal():
return meal(_that.plan,_that.dueMs,_that.missed);case NoticeDetail_Client():
return client(_that.client,_that.role);case NoticeDetail_UpdateFailed():
return updateFailed(_that.stop);case NoticeDetail_CannotFeed():
return cannotFeed(_that.reason);case NoticeDetail_FoodLikelyOut():
return foodLikelyOut(_that.confirmed);case NoticeDetail_DesiccantDue():
return desiccantDue(_that.days);case NoticeDetail_Unreachable():
return unreachable(_that.sinceMs);case NoticeDetail_Plain():
return plain();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( int feed,  int? plan,  int requested,  int completed,  FeedOutcome? outcome,  FeedReason reason)?  feed,TResult? Function( int plan,  PlatformInt64 dueMs,  MissedReason? missed)?  meal,TResult? Function( String client,  ClientRole? role)?  client,TResult? Function( UpdateStop stop)?  updateFailed,TResult? Function( Blocked reason)?  cannotFeed,TResult? Function( int confirmed)?  foodLikelyOut,TResult? Function( int days)?  desiccantDue,TResult? Function( PlatformInt64 sinceMs)?  unreachable,TResult? Function()?  plain,}) {final _that = this;
switch (_that) {
case NoticeDetail_Feed() when feed != null:
return feed(_that.feed,_that.plan,_that.requested,_that.completed,_that.outcome,_that.reason);case NoticeDetail_Meal() when meal != null:
return meal(_that.plan,_that.dueMs,_that.missed);case NoticeDetail_Client() when client != null:
return client(_that.client,_that.role);case NoticeDetail_UpdateFailed() when updateFailed != null:
return updateFailed(_that.stop);case NoticeDetail_CannotFeed() when cannotFeed != null:
return cannotFeed(_that.reason);case NoticeDetail_FoodLikelyOut() when foodLikelyOut != null:
return foodLikelyOut(_that.confirmed);case NoticeDetail_DesiccantDue() when desiccantDue != null:
return desiccantDue(_that.days);case NoticeDetail_Unreachable() when unreachable != null:
return unreachable(_that.sinceMs);case NoticeDetail_Plain() when plain != null:
return plain();case _:
  return null;

}
}

}

/// @nodoc


class NoticeDetail_Feed extends NoticeDetail {
  const NoticeDetail_Feed({required this.feed, this.plan, required this.requested, required this.completed, this.outcome, required this.reason}): super._();
  

 final  int feed;
 final  int? plan;
 final  int requested;
 final  int completed;
 final  FeedOutcome? outcome;
 final  FeedReason reason;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeDetail_FeedCopyWith<NoticeDetail_Feed> get copyWith => _$NoticeDetail_FeedCopyWithImpl<NoticeDetail_Feed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail_Feed&&(identical(other.feed, feed) || other.feed == feed)&&(identical(other.plan, plan) || other.plan == plan)&&(identical(other.requested, requested) || other.requested == requested)&&(identical(other.completed, completed) || other.completed == completed)&&(identical(other.outcome, outcome) || other.outcome == outcome)&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode {
    return Object.hash(runtimeType,feed,plan,requested,completed,outcome,reason);
}

@override
String toString() {
    return 'NoticeDetail.feed(feed: $feed, plan: $plan, requested: $requested, completed: $completed, outcome: $outcome, reason: $reason)';
}


}

/// @nodoc
abstract mixin class $NoticeDetail_FeedCopyWith<$Res> implements $NoticeDetailCopyWith<$Res> {
  factory $NoticeDetail_FeedCopyWith(NoticeDetail_Feed value, $Res Function(NoticeDetail_Feed) _then) = _$NoticeDetail_FeedCopyWithImpl;
@useResult
$Res call({
 int feed, int? plan, int requested, int completed, FeedOutcome? outcome, FeedReason reason
});




}
/// @nodoc
class _$NoticeDetail_FeedCopyWithImpl<$Res>
    implements $NoticeDetail_FeedCopyWith<$Res> {
  _$NoticeDetail_FeedCopyWithImpl(this._self, this._then);

  final NoticeDetail_Feed _self;
  final $Res Function(NoticeDetail_Feed) _then;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? feed = null,Object? plan = freezed,Object? requested = null,Object? completed = null,Object? outcome = freezed,Object? reason = null,}) {
  return _then(NoticeDetail_Feed(
feed: null == feed ? _self.feed : feed // ignore: cast_nullable_to_non_nullable
as int,plan: freezed == plan ? _self.plan : plan // ignore: cast_nullable_to_non_nullable
as int?,requested: null == requested ? _self.requested : requested // ignore: cast_nullable_to_non_nullable
as int,completed: null == completed ? _self.completed : completed // ignore: cast_nullable_to_non_nullable
as int,outcome: freezed == outcome ? _self.outcome : outcome // ignore: cast_nullable_to_non_nullable
as FeedOutcome?,reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as FeedReason,
  ));
}


}

/// @nodoc


class NoticeDetail_Meal extends NoticeDetail {
  const NoticeDetail_Meal({required this.plan, required this.dueMs, this.missed}): super._();
  

 final  int plan;
 final  PlatformInt64 dueMs;
 final  MissedReason? missed;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeDetail_MealCopyWith<NoticeDetail_Meal> get copyWith => _$NoticeDetail_MealCopyWithImpl<NoticeDetail_Meal>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail_Meal&&(identical(other.plan, plan) || other.plan == plan)&&(identical(other.dueMs, dueMs) || other.dueMs == dueMs)&&(identical(other.missed, missed) || other.missed == missed));
}


@override
int get hashCode {
    return Object.hash(runtimeType,plan,dueMs,missed);
}

@override
String toString() {
    return 'NoticeDetail.meal(plan: $plan, dueMs: $dueMs, missed: $missed)';
}


}

/// @nodoc
abstract mixin class $NoticeDetail_MealCopyWith<$Res> implements $NoticeDetailCopyWith<$Res> {
  factory $NoticeDetail_MealCopyWith(NoticeDetail_Meal value, $Res Function(NoticeDetail_Meal) _then) = _$NoticeDetail_MealCopyWithImpl;
@useResult
$Res call({
 int plan, PlatformInt64 dueMs, MissedReason? missed
});


$MissedReasonCopyWith<$Res>? get missed;

}
/// @nodoc
class _$NoticeDetail_MealCopyWithImpl<$Res>
    implements $NoticeDetail_MealCopyWith<$Res> {
  _$NoticeDetail_MealCopyWithImpl(this._self, this._then);

  final NoticeDetail_Meal _self;
  final $Res Function(NoticeDetail_Meal) _then;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? plan = null,Object? dueMs = null,Object? missed = freezed,}) {
  return _then(NoticeDetail_Meal(
plan: null == plan ? _self.plan : plan // ignore: cast_nullable_to_non_nullable
as int,dueMs: null == dueMs ? _self.dueMs : dueMs // ignore: cast_nullable_to_non_nullable
as PlatformInt64,missed: freezed == missed ? _self.missed : missed // ignore: cast_nullable_to_non_nullable
as MissedReason?,
  ));
}

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@override
@pragma('vm:prefer-inline')
$MissedReasonCopyWith<$Res>? get missed {
    if (_self.missed == null) {
    return null;
  }

  return $MissedReasonCopyWith<$Res>(_self.missed!, (value) {
    return _then(_self.copyWith(missed: value));
  });
}
}

/// @nodoc


class NoticeDetail_Client extends NoticeDetail {
  const NoticeDetail_Client({required this.client, this.role}): super._();
  

/// The client's fingerprint, hex.
 final  String client;
 final  ClientRole? role;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeDetail_ClientCopyWith<NoticeDetail_Client> get copyWith => _$NoticeDetail_ClientCopyWithImpl<NoticeDetail_Client>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail_Client&&(identical(other.client, client) || other.client == client)&&(identical(other.role, role) || other.role == role));
}


@override
int get hashCode {
    return Object.hash(runtimeType,client,role);
}

@override
String toString() {
    return 'NoticeDetail.client(client: $client, role: $role)';
}


}

/// @nodoc
abstract mixin class $NoticeDetail_ClientCopyWith<$Res> implements $NoticeDetailCopyWith<$Res> {
  factory $NoticeDetail_ClientCopyWith(NoticeDetail_Client value, $Res Function(NoticeDetail_Client) _then) = _$NoticeDetail_ClientCopyWithImpl;
@useResult
$Res call({
 String client, ClientRole? role
});




}
/// @nodoc
class _$NoticeDetail_ClientCopyWithImpl<$Res>
    implements $NoticeDetail_ClientCopyWith<$Res> {
  _$NoticeDetail_ClientCopyWithImpl(this._self, this._then);

  final NoticeDetail_Client _self;
  final $Res Function(NoticeDetail_Client) _then;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? client = null,Object? role = freezed,}) {
  return _then(NoticeDetail_Client(
client: null == client ? _self.client : client // ignore: cast_nullable_to_non_nullable
as String,role: freezed == role ? _self.role : role // ignore: cast_nullable_to_non_nullable
as ClientRole?,
  ));
}


}

/// @nodoc


class NoticeDetail_UpdateFailed extends NoticeDetail {
  const NoticeDetail_UpdateFailed({required this.stop}): super._();
  

 final  UpdateStop stop;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeDetail_UpdateFailedCopyWith<NoticeDetail_UpdateFailed> get copyWith => _$NoticeDetail_UpdateFailedCopyWithImpl<NoticeDetail_UpdateFailed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail_UpdateFailed&&(identical(other.stop, stop) || other.stop == stop));
}


@override
int get hashCode {
    return Object.hash(runtimeType,stop);
}

@override
String toString() {
    return 'NoticeDetail.updateFailed(stop: $stop)';
}


}

/// @nodoc
abstract mixin class $NoticeDetail_UpdateFailedCopyWith<$Res> implements $NoticeDetailCopyWith<$Res> {
  factory $NoticeDetail_UpdateFailedCopyWith(NoticeDetail_UpdateFailed value, $Res Function(NoticeDetail_UpdateFailed) _then) = _$NoticeDetail_UpdateFailedCopyWithImpl;
@useResult
$Res call({
 UpdateStop stop
});




}
/// @nodoc
class _$NoticeDetail_UpdateFailedCopyWithImpl<$Res>
    implements $NoticeDetail_UpdateFailedCopyWith<$Res> {
  _$NoticeDetail_UpdateFailedCopyWithImpl(this._self, this._then);

  final NoticeDetail_UpdateFailed _self;
  final $Res Function(NoticeDetail_UpdateFailed) _then;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? stop = null,}) {
  return _then(NoticeDetail_UpdateFailed(
stop: null == stop ? _self.stop : stop // ignore: cast_nullable_to_non_nullable
as UpdateStop,
  ));
}


}

/// @nodoc


class NoticeDetail_CannotFeed extends NoticeDetail {
  const NoticeDetail_CannotFeed({required this.reason}): super._();
  

 final  Blocked reason;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeDetail_CannotFeedCopyWith<NoticeDetail_CannotFeed> get copyWith => _$NoticeDetail_CannotFeedCopyWithImpl<NoticeDetail_CannotFeed>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail_CannotFeed&&(identical(other.reason, reason) || other.reason == reason));
}


@override
int get hashCode {
    return Object.hash(runtimeType,reason);
}

@override
String toString() {
    return 'NoticeDetail.cannotFeed(reason: $reason)';
}


}

/// @nodoc
abstract mixin class $NoticeDetail_CannotFeedCopyWith<$Res> implements $NoticeDetailCopyWith<$Res> {
  factory $NoticeDetail_CannotFeedCopyWith(NoticeDetail_CannotFeed value, $Res Function(NoticeDetail_CannotFeed) _then) = _$NoticeDetail_CannotFeedCopyWithImpl;
@useResult
$Res call({
 Blocked reason
});




}
/// @nodoc
class _$NoticeDetail_CannotFeedCopyWithImpl<$Res>
    implements $NoticeDetail_CannotFeedCopyWith<$Res> {
  _$NoticeDetail_CannotFeedCopyWithImpl(this._self, this._then);

  final NoticeDetail_CannotFeed _self;
  final $Res Function(NoticeDetail_CannotFeed) _then;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? reason = null,}) {
  return _then(NoticeDetail_CannotFeed(
reason: null == reason ? _self.reason : reason // ignore: cast_nullable_to_non_nullable
as Blocked,
  ));
}


}

/// @nodoc


class NoticeDetail_FoodLikelyOut extends NoticeDetail {
  const NoticeDetail_FoodLikelyOut({required this.confirmed}): super._();
  

 final  int confirmed;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeDetail_FoodLikelyOutCopyWith<NoticeDetail_FoodLikelyOut> get copyWith => _$NoticeDetail_FoodLikelyOutCopyWithImpl<NoticeDetail_FoodLikelyOut>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail_FoodLikelyOut&&(identical(other.confirmed, confirmed) || other.confirmed == confirmed));
}


@override
int get hashCode {
    return Object.hash(runtimeType,confirmed);
}

@override
String toString() {
    return 'NoticeDetail.foodLikelyOut(confirmed: $confirmed)';
}


}

/// @nodoc
abstract mixin class $NoticeDetail_FoodLikelyOutCopyWith<$Res> implements $NoticeDetailCopyWith<$Res> {
  factory $NoticeDetail_FoodLikelyOutCopyWith(NoticeDetail_FoodLikelyOut value, $Res Function(NoticeDetail_FoodLikelyOut) _then) = _$NoticeDetail_FoodLikelyOutCopyWithImpl;
@useResult
$Res call({
 int confirmed
});




}
/// @nodoc
class _$NoticeDetail_FoodLikelyOutCopyWithImpl<$Res>
    implements $NoticeDetail_FoodLikelyOutCopyWith<$Res> {
  _$NoticeDetail_FoodLikelyOutCopyWithImpl(this._self, this._then);

  final NoticeDetail_FoodLikelyOut _self;
  final $Res Function(NoticeDetail_FoodLikelyOut) _then;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? confirmed = null,}) {
  return _then(NoticeDetail_FoodLikelyOut(
confirmed: null == confirmed ? _self.confirmed : confirmed // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class NoticeDetail_DesiccantDue extends NoticeDetail {
  const NoticeDetail_DesiccantDue({required this.days}): super._();
  

 final  int days;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeDetail_DesiccantDueCopyWith<NoticeDetail_DesiccantDue> get copyWith => _$NoticeDetail_DesiccantDueCopyWithImpl<NoticeDetail_DesiccantDue>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail_DesiccantDue&&(identical(other.days, days) || other.days == days));
}


@override
int get hashCode {
    return Object.hash(runtimeType,days);
}

@override
String toString() {
    return 'NoticeDetail.desiccantDue(days: $days)';
}


}

/// @nodoc
abstract mixin class $NoticeDetail_DesiccantDueCopyWith<$Res> implements $NoticeDetailCopyWith<$Res> {
  factory $NoticeDetail_DesiccantDueCopyWith(NoticeDetail_DesiccantDue value, $Res Function(NoticeDetail_DesiccantDue) _then) = _$NoticeDetail_DesiccantDueCopyWithImpl;
@useResult
$Res call({
 int days
});




}
/// @nodoc
class _$NoticeDetail_DesiccantDueCopyWithImpl<$Res>
    implements $NoticeDetail_DesiccantDueCopyWith<$Res> {
  _$NoticeDetail_DesiccantDueCopyWithImpl(this._self, this._then);

  final NoticeDetail_DesiccantDue _self;
  final $Res Function(NoticeDetail_DesiccantDue) _then;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? days = null,}) {
  return _then(NoticeDetail_DesiccantDue(
days: null == days ? _self.days : days // ignore: cast_nullable_to_non_nullable
as int,
  ));
}


}

/// @nodoc


class NoticeDetail_Unreachable extends NoticeDetail {
  const NoticeDetail_Unreachable({required this.sinceMs}): super._();
  

 final  PlatformInt64 sinceMs;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$NoticeDetail_UnreachableCopyWith<NoticeDetail_Unreachable> get copyWith => _$NoticeDetail_UnreachableCopyWithImpl<NoticeDetail_Unreachable>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail_Unreachable&&(identical(other.sinceMs, sinceMs) || other.sinceMs == sinceMs));
}


@override
int get hashCode {
    return Object.hash(runtimeType,sinceMs);
}

@override
String toString() {
    return 'NoticeDetail.unreachable(sinceMs: $sinceMs)';
}


}

/// @nodoc
abstract mixin class $NoticeDetail_UnreachableCopyWith<$Res> implements $NoticeDetailCopyWith<$Res> {
  factory $NoticeDetail_UnreachableCopyWith(NoticeDetail_Unreachable value, $Res Function(NoticeDetail_Unreachable) _then) = _$NoticeDetail_UnreachableCopyWithImpl;
@useResult
$Res call({
 PlatformInt64 sinceMs
});




}
/// @nodoc
class _$NoticeDetail_UnreachableCopyWithImpl<$Res>
    implements $NoticeDetail_UnreachableCopyWith<$Res> {
  _$NoticeDetail_UnreachableCopyWithImpl(this._self, this._then);

  final NoticeDetail_Unreachable _self;
  final $Res Function(NoticeDetail_Unreachable) _then;

/// Create a copy of NoticeDetail
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? sinceMs = null,}) {
  return _then(NoticeDetail_Unreachable(
sinceMs: null == sinceMs ? _self.sinceMs : sinceMs // ignore: cast_nullable_to_non_nullable
as PlatformInt64,
  ));
}


}

/// @nodoc


class NoticeDetail_Plain extends NoticeDetail {
  const NoticeDetail_Plain(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is NoticeDetail_Plain);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'NoticeDetail.plain()';
}


}




/// @nodoc
mixin _$PhoneNetwork {





@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is PhoneNetwork);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'PhoneNetwork()';
}


}

/// @nodoc
class $PhoneNetworkCopyWith<$Res>  {
$PhoneNetworkCopyWith(PhoneNetwork _, $Res Function(PhoneNetwork) __);
}


/// Adds pattern-matching-related methods to [PhoneNetwork].
extension PhoneNetworkPatterns on PhoneNetwork {
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

@optionalTypeArgs TResult maybeMap<TResult extends Object?>({TResult Function( PhoneNetwork_Wifi value)?  wifi,TResult Function( PhoneNetwork_Elsewhere value)?  elsewhere,required TResult orElse(),}){
final _that = this;
switch (_that) {
case PhoneNetwork_Wifi() when wifi != null:
return wifi(_that);case PhoneNetwork_Elsewhere() when elsewhere != null:
return elsewhere(_that);case _:
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

@optionalTypeArgs TResult map<TResult extends Object?>({required TResult Function( PhoneNetwork_Wifi value)  wifi,required TResult Function( PhoneNetwork_Elsewhere value)  elsewhere,}){
final _that = this;
switch (_that) {
case PhoneNetwork_Wifi():
return wifi(_that);case PhoneNetwork_Elsewhere():
return elsewhere(_that);}
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

@optionalTypeArgs TResult? mapOrNull<TResult extends Object?>({TResult? Function( PhoneNetwork_Wifi value)?  wifi,TResult? Function( PhoneNetwork_Elsewhere value)?  elsewhere,}){
final _that = this;
switch (_that) {
case PhoneNetwork_Wifi() when wifi != null:
return wifi(_that);case PhoneNetwork_Elsewhere() when elsewhere != null:
return elsewhere(_that);case _:
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

@optionalTypeArgs TResult maybeWhen<TResult extends Object?>({TResult Function( String gateway,  String subnet)?  wifi,TResult Function()?  elsewhere,required TResult orElse(),}) {final _that = this;
switch (_that) {
case PhoneNetwork_Wifi() when wifi != null:
return wifi(_that.gateway,_that.subnet);case PhoneNetwork_Elsewhere() when elsewhere != null:
return elsewhere();case _:
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

@optionalTypeArgs TResult when<TResult extends Object?>({required TResult Function( String gateway,  String subnet)  wifi,required TResult Function()  elsewhere,}) {final _that = this;
switch (_that) {
case PhoneNetwork_Wifi():
return wifi(_that.gateway,_that.subnet);case PhoneNetwork_Elsewhere():
return elsewhere();}
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

@optionalTypeArgs TResult? whenOrNull<TResult extends Object?>({TResult? Function( String gateway,  String subnet)?  wifi,TResult? Function()?  elsewhere,}) {final _that = this;
switch (_that) {
case PhoneNetwork_Wifi() when wifi != null:
return wifi(_that.gateway,_that.subnet);case PhoneNetwork_Elsewhere() when elsewhere != null:
return elsewhere();case _:
  return null;

}
}

}

/// @nodoc


class PhoneNetwork_Wifi extends PhoneNetwork {
  const PhoneNetwork_Wifi({required this.gateway, required this.subnet}): super._();
  

 final  String gateway;
 final  String subnet;

/// Create a copy of PhoneNetwork
/// with the given fields replaced by the non-null parameter values.
@JsonKey(includeFromJson: false, includeToJson: false)
@pragma('vm:prefer-inline')
$PhoneNetwork_WifiCopyWith<PhoneNetwork_Wifi> get copyWith => _$PhoneNetwork_WifiCopyWithImpl<PhoneNetwork_Wifi>(this, _$identity);



@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is PhoneNetwork_Wifi&&(identical(other.gateway, gateway) || other.gateway == gateway)&&(identical(other.subnet, subnet) || other.subnet == subnet));
}


@override
int get hashCode {
    return Object.hash(runtimeType,gateway,subnet);
}

@override
String toString() {
    return 'PhoneNetwork.wifi(gateway: $gateway, subnet: $subnet)';
}


}

/// @nodoc
abstract mixin class $PhoneNetwork_WifiCopyWith<$Res> implements $PhoneNetworkCopyWith<$Res> {
  factory $PhoneNetwork_WifiCopyWith(PhoneNetwork_Wifi value, $Res Function(PhoneNetwork_Wifi) _then) = _$PhoneNetwork_WifiCopyWithImpl;
@useResult
$Res call({
 String gateway, String subnet
});




}
/// @nodoc
class _$PhoneNetwork_WifiCopyWithImpl<$Res>
    implements $PhoneNetwork_WifiCopyWith<$Res> {
  _$PhoneNetwork_WifiCopyWithImpl(this._self, this._then);

  final PhoneNetwork_Wifi _self;
  final $Res Function(PhoneNetwork_Wifi) _then;

/// Create a copy of PhoneNetwork
/// with the given fields replaced by the non-null parameter values.
@pragma('vm:prefer-inline') $Res call({Object? gateway = null,Object? subnet = null,}) {
  return _then(PhoneNetwork_Wifi(
gateway: null == gateway ? _self.gateway : gateway // ignore: cast_nullable_to_non_nullable
as String,subnet: null == subnet ? _self.subnet : subnet // ignore: cast_nullable_to_non_nullable
as String,
  ));
}


}

/// @nodoc


class PhoneNetwork_Elsewhere extends PhoneNetwork {
  const PhoneNetwork_Elsewhere(): super._();
  






@override
bool operator ==(Object other) {
    return identical(this, other) || (other.runtimeType == runtimeType&&other is PhoneNetwork_Elsewhere);
}


@override
int get hashCode => runtimeType.hashCode;

@override
String toString() {
    return 'PhoneNetwork.elsewhere()';
}


}




// dart format on
