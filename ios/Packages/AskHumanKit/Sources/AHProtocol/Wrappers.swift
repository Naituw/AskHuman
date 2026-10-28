import Foundation

// Property wrappers that reproduce the serde attributes of `src-tauri/src/ios/wire.rs`:
// `#[serde(default, skip_serializing_if = "is_false")]` and friends. The `KeyedEncodingContainer`
// / `KeyedDecodingContainer` overloads make the synthesized Codable code omit or default the key.

/// A boolean that is absent from the wire when `false`.
@propertyWrapper
public struct OmitFalse: Codable, Hashable, Sendable {
    public var wrappedValue: Bool
    public init(wrappedValue: Bool) { self.wrappedValue = wrappedValue }
    public init(from decoder: Decoder) throws {
        wrappedValue = try decoder.singleValueContainer().decode(Bool.self)
    }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(wrappedValue)
    }
}

/// A counter that is absent from the wire when zero.
@propertyWrapper
public struct OmitZero: Codable, Hashable, Sendable {
    public var wrappedValue: UInt64
    public init(wrappedValue: UInt64) { self.wrappedValue = wrappedValue }
    public init(from decoder: Decoder) throws {
        wrappedValue = try decoder.singleValueContainer().decode(UInt64.self)
    }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(wrappedValue)
    }
}

/// An array that decodes to empty when the key is absent (always encoded).
@propertyWrapper
public struct DefaultEmpty<Element: Codable & Hashable & Sendable>: Codable, Hashable, Sendable {
    public var wrappedValue: [Element]
    public init(wrappedValue: [Element]) { self.wrappedValue = wrappedValue }
    public init(from decoder: Decoder) throws {
        wrappedValue = try decoder.singleValueContainer().decode([Element].self)
    }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(wrappedValue)
    }
}

/// An array that is absent from the wire when empty (`skip_serializing_if = "Vec::is_empty"`) and decodes
/// to empty when the key is absent.
@propertyWrapper
public struct OmitEmpty<Element: Codable & Hashable & Sendable>: Codable, Hashable, Sendable {
    public var wrappedValue: [Element]
    public init(wrappedValue: [Element]) { self.wrappedValue = wrappedValue }
    public init(from decoder: Decoder) throws {
        wrappedValue = try decoder.singleValueContainer().decode([Element].self)
    }
    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(wrappedValue)
    }
}

extension KeyedEncodingContainer {
    public mutating func encode<E>(_ value: OmitEmpty<E>, forKey key: Key) throws {
        if !value.wrappedValue.isEmpty { try encode(value.wrappedValue, forKey: key) }
    }

    public mutating func encode(_ value: OmitFalse, forKey key: Key) throws {
        if value.wrappedValue { try encode(true, forKey: key) }
    }

    public mutating func encode(_ value: OmitZero, forKey key: Key) throws {
        if value.wrappedValue != 0 { try encode(value.wrappedValue, forKey: key) }
    }
}

extension KeyedDecodingContainer {
    public func decode(_ type: OmitFalse.Type, forKey key: Key) throws -> OmitFalse {
        OmitFalse(wrappedValue: try decodeIfPresent(Bool.self, forKey: key) ?? false)
    }

    public func decode(_ type: OmitZero.Type, forKey key: Key) throws -> OmitZero {
        OmitZero(wrappedValue: try decodeIfPresent(UInt64.self, forKey: key) ?? 0)
    }

    public func decode<E>(_ type: DefaultEmpty<E>.Type, forKey key: Key) throws -> DefaultEmpty<E> {
        DefaultEmpty(wrappedValue: try decodeIfPresent([E].self, forKey: key) ?? [])
    }

    public func decode<E>(_ type: OmitEmpty<E>.Type, forKey key: Key) throws -> OmitEmpty<E> {
        OmitEmpty(wrappedValue: try decodeIfPresent([E].self, forKey: key) ?? [])
    }
}
