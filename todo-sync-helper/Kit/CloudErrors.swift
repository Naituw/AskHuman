import CloudKit
import Foundation

/// CloudKit failure -> the small vocabulary the core understands (`transport::NetError`).
enum CloudErrors {
    /// Retry hint when CloudKit throttles us without saying for how long.
    static let defaultRetryAfterMs = 30_000

    static func failure(from error: Error) -> RpcFailure {
        if let failure = error as? RpcFailure { return failure }
        guard let ck = error as? CKError else {
            let ns = error as NSError
            if ns.domain == NSURLErrorDomain {
                return RpcFailure(code: RpcCode.offline, message: "network: \(ns.code)")
            }
            return RpcFailure(code: RpcCode.other, message: "\(ns.domain) \(ns.code)")
        }
        return failure(code: ck.code, retryAfter: ck.retryAfterSeconds, partial: ck.partialErrorsByItemID)
    }

    /// Split out so tests can drive it without building `CKError` values.
    static func failure(code: CKError.Code, retryAfter: Double?, partial: [AnyHashable: Error]? = nil) -> RpcFailure {
        switch code {
        case .networkUnavailable, .networkFailure:
            return RpcFailure(code: RpcCode.offline, message: name(of: code))
        case .serviceUnavailable, .requestRateLimited, .zoneBusy:
            let ms = retryAfter.map { Int($0 * 1000) } ?? defaultRetryAfterMs
            return RpcFailure(code: RpcCode.rateLimited, retryAfterMs: max(ms, 1000), message: name(of: code))
        case .notAuthenticated:
            return RpcFailure(code: RpcCode.notAuthenticated, message: name(of: code))
        case .accountTemporarilyUnavailable:
            // Signed in but iCloud is not ready (updating, restoring): a retry later, not a pause.
            return RpcFailure(code: RpcCode.offline, message: name(of: code))
        case .zoneNotFound, .userDeletedZone:
            return RpcFailure(code: RpcCode.zoneNotFound, message: name(of: code))
        case .changeTokenExpired:
            return RpcFailure(code: RpcCode.tokenExpired, message: name(of: code))
        case .quotaExceeded:
            return RpcFailure(code: RpcCode.quota, message: name(of: code))
        case .badContainer:
            // A container created minutes ago answers "bad container" until Apple's servers catch up.
            return RpcFailure(code: RpcCode.offline, message: name(of: code))
        case .partialFailure:
            // Report the most actionable member. Order: things that pause sync first, then transient.
            let inner = (partial ?? [:]).values.compactMap { $0 as? CKError }.map { failure(from: $0) }
            let rank = [RpcCode.notAuthenticated, RpcCode.zoneNotFound, RpcCode.quota, RpcCode.tokenExpired, RpcCode.rateLimited, RpcCode.offline]
            for wanted in rank {
                if let hit = inner.first(where: { $0.code == wanted }) { return hit }
            }
            return inner.first ?? RpcFailure(code: RpcCode.other, message: name(of: code))
        default:
            return RpcFailure(code: RpcCode.other, message: name(of: code))
        }
    }

    /// Per-record save failure, reported inside a `save` reply (the core only acts on "quotaExceeded").
    static func recordCode(from error: Error) -> String {
        guard let ck = error as? CKError else { return "error" }
        if ck.code == .quotaExceeded { return "quotaExceeded" }
        return name(of: ck.code)
    }

    static func name(of code: CKError.Code) -> String {
        switch code {
        case .internalError: return "internalError"
        case .partialFailure: return "partialFailure"
        case .networkUnavailable: return "networkUnavailable"
        case .networkFailure: return "networkFailure"
        case .badContainer: return "badContainer"
        case .serviceUnavailable: return "serviceUnavailable"
        case .requestRateLimited: return "requestRateLimited"
        case .missingEntitlement: return "missingEntitlement"
        case .notAuthenticated: return "notAuthenticated"
        case .permissionFailure: return "permissionFailure"
        case .unknownItem: return "unknownItem"
        case .invalidArguments: return "invalidArguments"
        case .serverRecordChanged: return "serverRecordChanged"
        case .serverRejectedRequest: return "serverRejectedRequest"
        case .assetFileNotFound: return "assetFileNotFound"
        case .assetFileModified: return "assetFileModified"
        case .incompatibleVersion: return "incompatibleVersion"
        case .constraintViolation: return "constraintViolation"
        case .operationCancelled: return "operationCancelled"
        case .changeTokenExpired: return "changeTokenExpired"
        case .batchRequestFailed: return "batchRequestFailed"
        case .zoneBusy: return "zoneBusy"
        case .badDatabase: return "badDatabase"
        case .quotaExceeded: return "quotaExceeded"
        case .zoneNotFound: return "zoneNotFound"
        case .limitExceeded: return "limitExceeded"
        case .userDeletedZone: return "userDeletedZone"
        case .tooManyParticipants: return "tooManyParticipants"
        case .alreadyShared: return "alreadyShared"
        case .referenceViolation: return "referenceViolation"
        case .managedAccountRestricted: return "managedAccountRestricted"
        case .participantMayNeedVerification: return "participantMayNeedVerification"
        case .serverResponseLost: return "serverResponseLost"
        case .assetNotAvailable: return "assetNotAvailable"
        case .accountTemporarilyUnavailable: return "accountTemporarilyUnavailable"
        default: return "ck\(code.rawValue)"
        }
    }
}
